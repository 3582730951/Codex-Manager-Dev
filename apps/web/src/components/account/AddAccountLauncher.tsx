"use client";

import {
  useEffect,
  useMemo,
  useRef,
  useState,
  useTransition
} from "react";
import { useRouter } from "next/navigation";

type LoginStartResult = {
  loginId: string;
  authUrl: string;
  redirectUri: string;
};

type LoginSession = {
  status: string;
  error?: string | null;
  importedAccountLabel?: string | null;
};

type ImportResult = {
  total: number;
  created: number;
  failed: number;
  errors?: string[];
};

type AddAccountLauncherProps = {
  callbackUrl: string;
  accountCount: number;
  tenantCount: number;
};

type ActiveTab = "login" | "callback" | "import";

function parseError(payload: unknown, fallback: string) {
  if (payload && typeof payload === "object") {
    const source = payload as { message?: unknown };
    if (typeof source.message === "string" && source.message.trim()) {
      return source.message.trim();
    }
  }
  return fallback;
}

async function readJson<T>(response: Response, fallback: string) {
  const payload = (await response.json().catch(() => null)) as unknown;
  if (!response.ok) {
    throw new Error(parseError(payload, fallback));
  }
  return payload as T;
}

function normalizeCallbackUrl(rawValue: string, callbackUrl: string) {
  const value = rawValue.trim();
  if (!value) {
    throw new Error("请先粘贴登录成功后的完整回调地址。");
  }

  const callbackBase = callbackUrl.split("?")[0] ?? callbackUrl;
  const buildUrl = (state: string, code: string) =>
    `${callbackBase}?state=${encodeURIComponent(state)}&code=${encodeURIComponent(code)}`;

  const fromParams = (params: URLSearchParams) => {
    const state = params.get("state")?.trim();
    const code = params.get("code")?.trim();
    if (state && code) {
      return buildUrl(state, code);
    }
    return null;
  };

  const candidates = [value];
  if (value.startsWith("?")) {
    candidates.push(`http://callback.local/${value}`);
  }
  if (value.startsWith("/")) {
    candidates.push(`http://callback.local${value}`);
  }

  for (const candidate of candidates) {
    try {
      const parsed = new URL(candidate);
      const normalized = fromParams(parsed.searchParams);
      if (normalized) {
        return normalized;
      }
    } catch {
      // fall through to loose parsing
    }
  }

  const queryIndex = value.indexOf("?");
  const looseQuery = queryIndex >= 0 ? value.slice(queryIndex + 1) : value;
  const normalized = fromParams(new URLSearchParams(looseQuery));
  if (normalized) {
    return normalized;
  }

  throw new Error("回调地址里缺少 state 或 code。请粘贴浏览器最终停留的完整地址。");
}

export function AddAccountLauncher({
  callbackUrl,
  accountCount,
  tenantCount
}: AddAccountLauncherProps) {
  const router = useRouter();
  const [open, setOpen] = useState(false);
  const [activeTab, setActiveTab] = useState<ActiveTab>("login");
  const [label, setLabel] = useState("");
  const [note, setNote] = useState("");
  const [callbackValue, setCallbackValue] = useState("");
  const [loginUrl, setLoginUrl] = useState("");
  const [loginId, setLoginId] = useState("");
  const [statusTone, setStatusTone] = useState<"neutral" | "ok" | "error">("neutral");
  const [statusMessage, setStatusMessage] = useState(
    "点击主按钮后，会直接打开 OpenAI 官方登录窗口。"
  );
  const [selectedFiles, setSelectedFiles] = useState<File[]>([]);
  const popupRef = useRef<Window | null>(null);
  const closeTimerRef = useRef<number | null>(null);
  const [isPending, startTransition] = useTransition();

  const statusLabel = useMemo(() => {
    if (tenantCount === 0) {
      return "首次授权会自动创建默认租户";
    }
    return `当前 ${tenantCount} 个租户，${accountCount} 个账号`;
  }, [accountCount, tenantCount]);

  const guide = useMemo(() => {
    if (activeTab === "login") {
      return {
        kicker: "主入口",
        title: "弹窗授权，一次完成",
        body: "这里是默认入口。点击后会直接打开 OpenAI 官方授权页，登录成功后自动回写主页面。",
        steps: [
          "点击“登录 OpenAI”，浏览器会打开官方授权页。",
          "在新窗口完成登录或授权，保持当前页面不关闭。",
          "回调页会自动把结果通知主页面，账号直接入池。"
        ]
      };
    }
    if (activeTab === "callback") {
      return {
        kicker: "兜底入口",
        title: "粘贴地址即可补导入",
        body: "如果浏览器没有自动回传，只要把最终地址完整粘贴回来，系统就能解析 state 和 code。",
        steps: [
          "回调地址里必须同时带 state 和 code。",
          "支持粘贴完整 URL，也支持只粘贴查询串。",
          "解析成功后会立刻导入账号并刷新工作台。"
        ]
      };
    }
    return {
      kicker: "批量入口",
      title: "账号文件统一导入",
      body: "适合已有 JSON、TXT 或日志文件的场景。系统会自动补默认租户并一次完成导入。",
      steps: [
        "选择一个或多个账号文件。",
        "支持 JSON、TXT、LOG 等文本格式。",
        "导入完成后会自动关闭弹层并刷新列表。"
      ]
    };
  }, [activeTab]);

  useEffect(() => {
    if (!open) {
      return undefined;
    }

    const previousBody = document.body.style.overflow;
    const previousHtml = document.documentElement.style.overflow;
    document.body.style.overflow = "hidden";
    document.documentElement.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = previousBody;
      document.documentElement.style.overflow = previousHtml;
    };
  }, [open]);

  useEffect(() => {
    if (!open) {
      return undefined;
    }

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setOpen(false);
        resetModal();
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [open]);

  useEffect(() => {
    function handleMessage(event: MessageEvent) {
      if (!event.data || typeof event.data !== "object") {
        return;
      }
      const payload = event.data as {
        type?: string;
        message?: string;
        importedLabel?: string;
      };
      if (payload.type === "codex-manager:login-complete") {
        finishSuccess(
          payload.importedLabel
            ? `已导入账号 ${payload.importedLabel}`
            : payload.message || "OpenAI 账号已导入控制面。"
        );
      }
      if (payload.type === "codex-manager:login-error") {
        setStatusTone("error");
        setStatusMessage(payload.message || "授权回调解析失败。");
        setActiveTab("callback");
      }
    }

    window.addEventListener("message", handleMessage);
    return () => window.removeEventListener("message", handleMessage);
  }, []);

  useEffect(() => {
    if (!open || !loginId) {
      return undefined;
    }

    const timer = window.setInterval(async () => {
      try {
        const response = await fetch(`/api/account-intake/login/${loginId}`, {
          cache: "no-store"
        });
        const session = await readJson<LoginSession>(response, "读取登录状态失败。");
        const status = String(session.status || "").trim().toLowerCase();
        if (status === "success") {
          finishSuccess(
            session.importedAccountLabel
              ? `已导入账号 ${session.importedAccountLabel}`
              : "OpenAI 账号已导入控制面。"
          );
        }
        if (status === "failed") {
          setStatusTone("error");
          setStatusMessage(session.error?.trim() || "授权失败，请重试或改用回调解析。");
          setActiveTab("callback");
          clearLoginSession();
        }
      } catch {
        // keep polling; callback page can still push a postMessage on success
      }
    }, 1500);

    return () => window.clearInterval(timer);
  }, [loginId, open]);

  function clearLoginSession() {
    setLoginId("");
    if (popupRef.current && !popupRef.current.closed) {
      popupRef.current.close();
    }
    popupRef.current = null;
  }

  function resetModal() {
    clearLoginSession();
    if (closeTimerRef.current) {
      window.clearTimeout(closeTimerRef.current);
      closeTimerRef.current = null;
    }
    setActiveTab("login");
    setLabel("");
    setNote("");
    setCallbackValue("");
    setLoginUrl("");
    setSelectedFiles([]);
    setStatusTone("neutral");
    setStatusMessage("点击主按钮后，会直接打开 OpenAI 官方登录窗口。");
  }

  function finishSuccess(message: string) {
    clearLoginSession();
    setStatusTone("ok");
    setStatusMessage(message);
    startTransition(() => {
      router.refresh();
    });
    closeTimerRef.current = window.setTimeout(() => {
      closeTimerRef.current = null;
      setOpen(false);
      resetModal();
    }, 900);
  }

  async function handleStartLogin() {
    setStatusTone("neutral");
    setStatusMessage(
      tenantCount === 0
        ? "正在生成授权窗口，默认租户会在首次成功后自动创建。"
        : "正在生成 OpenAI 授权窗口..."
    );

    try {
      const response = await fetch("/api/account-intake/login/start", {
        method: "POST",
        headers: {
          "content-type": "application/json"
        },
        body: JSON.stringify({
          label,
          note,
          redirectUri: callbackUrl
        })
      });
      const result = await readJson<LoginStartResult>(
        response,
        "OpenAI 授权地址生成失败。"
      );
      setLoginId(result.loginId);
      setLoginUrl(result.authUrl);
      setStatusMessage("授权窗口已打开，完成登录后会自动导入账号。");

      const popup = window.open(
        result.authUrl,
        "codex-manager-openai-login",
        "popup=yes,width=560,height=760,resizable=yes,scrollbars=yes"
      );
      if (!popup) {
        setStatusTone("error");
        setStatusMessage("浏览器拦截了弹窗。请使用下方“打开当前授权页”继续。");
        return;
      }

      popupRef.current = popup;
      popup.focus();
    } catch (error) {
      setStatusTone("error");
      setStatusMessage(error instanceof Error ? error.message : "OpenAI 授权启动失败。");
    }
  }

  async function handleManualCallback() {
    try {
      const normalizedCallbackUrl = normalizeCallbackUrl(callbackValue, callbackUrl);
      setStatusTone("neutral");
      setStatusMessage("正在解析回调并导入账号...");
      const response = await fetch("/api/account-intake/login/complete", {
        method: "POST",
        headers: {
          "content-type": "application/json"
        },
        body: JSON.stringify({
          callbackUrl: normalizedCallbackUrl
        })
      });
      const payload = await readJson<{
        session?: { importedAccountLabel?: string | null };
      }>(response, "回调解析失败。");
      finishSuccess(
        payload.session?.importedAccountLabel
          ? `已导入账号 ${payload.session.importedAccountLabel}`
          : "OpenAI 账号已导入控制面。"
      );
    } catch (error) {
      setStatusTone("error");
      setStatusMessage(error instanceof Error ? error.message : "回调解析失败。");
    }
  }

  async function handleImportFiles() {
    if (selectedFiles.length === 0) {
      setStatusTone("error");
      setStatusMessage("请先选择账号文件。");
      return;
    }

    setStatusTone("neutral");
    setStatusMessage("正在导入账号文件...");
    try {
      const contents = await Promise.all(selectedFiles.map((file) => file.text()));
      const response = await fetch("/api/account-intake/import", {
        method: "POST",
        headers: {
          "content-type": "application/json"
        },
        body: JSON.stringify({ contents })
      });
      const result = await readJson<ImportResult>(response, "账号导入失败。");
      finishSuccess(
        result.failed > 0
          ? `导入完成，成功 ${result.created} 条，失败 ${result.failed} 条。`
          : `导入完成，已新增 ${result.created} 个账号。`
      );
    } catch (error) {
      setStatusTone("error");
      setStatusMessage(error instanceof Error ? error.message : "账号导入失败。");
    }
  }

  return (
    <>
      <div className="intake-toolbar">
        <div className="intake-toolbar-copy">
          <strong>添加账号</strong>
          <span>{statusLabel}</span>
        </div>
        <button
          className="button primary launcher-button"
          onClick={() => setOpen(true)}
          type="button"
        >
          <span>+ 添加账号</span>
        </button>
      </div>

      <div className="launcher-grid">
        <article className="launcher-card launcher-card-primary">
          <span className="launcher-mark">01</span>
          <strong>一键登录接入</strong>
          <p>主入口直接打开 OpenAI 官方登录页，成功后自动写入账号工作台。</p>
          <button
            className="button primary"
            onClick={() => {
              setOpen(true);
              setActiveTab("login");
            }}
            type="button"
          >
            <span>登录 OpenAI</span>
          </button>
        </article>

        <article className="launcher-card">
          <span className="launcher-mark">02</span>
          <strong>粘贴回调补导入</strong>
          <p>适合浏览器没有自动关闭的情况，直接粘贴最终地址即可恢复导入。</p>
          <button
            className="button ghost"
            onClick={() => {
              setOpen(true);
              setActiveTab("callback");
            }}
            type="button"
          >
            <span>粘贴回调</span>
          </button>
        </article>

        <article className="launcher-card">
          <span className="launcher-mark">03</span>
          <strong>文件批量导入</strong>
          <p>选择 JSON、TXT 或日志文件，系统自动归并并写入当前账号池。</p>
          <button
            className="button ghost"
            onClick={() => {
              setOpen(true);
              setActiveTab("import");
            }}
            type="button"
          >
            <span>选择文件</span>
          </button>
        </article>
      </div>

      {open ? (
        <div
          className="modal-overlay"
          onClick={() => {
            setOpen(false);
            resetModal();
          }}
          role="presentation"
        >
          <section
            aria-modal="true"
            className="modal-panel"
            onClick={(event) => event.stopPropagation()}
            role="dialog"
          >
            <header className="modal-head modal-head-rich">
              <div className="modal-head-copy">
                <p className="section-kicker">账号接入</p>
                <h3>一键接入 OpenAI 账号</h3>
                <p className="modal-intro">
                  默认走官方授权页。成功后会回到当前站点的回调页，再自动把账号写入控制面。
                </p>
              </div>
              <div className="modal-head-actions">
                <span className="modal-chip">
                  {tenantCount === 0 ? "默认租户自动创建" : "沿用当前租户"}
                </span>
                <span className="modal-chip">
                  {loginId ? "授权会话进行中" : "等待授权"}
                </span>
                <button
                  className="button ghost modal-close"
                  onClick={() => {
                    setOpen(false);
                    resetModal();
                  }}
                  type="button"
                >
                  <span>关闭</span>
                </button>
              </div>
            </header>

            <div className={`status-banner ${statusTone}`}>
              <strong>
                {statusTone === "ok" ? "已完成" : statusTone === "error" ? "需要处理" : "准备中"}
              </strong>
              <p>{statusMessage}</p>
            </div>

            <div className="modal-tabs" role="tablist">
              {[
                ["login", "登录授权"],
                ["callback", "回调解析"],
                ["import", "文件导入"]
              ].map(([key, title]) => (
                <button
                  aria-selected={activeTab === key}
                  className={`modal-tab ${activeTab === key ? "active" : ""}`}
                  key={key}
                  onClick={() => setActiveTab(key as ActiveTab)}
                  role="tab"
                  type="button"
                >
                  {title}
                </button>
              ))}
            </div>

            <div className="modal-stage">
              <aside className="modal-aside">
                <article className="modal-guide-card">
                  <p className="section-kicker">{guide.kicker}</p>
                  <strong>{guide.title}</strong>
                  <p>{guide.body}</p>
                </article>

                <div className="modal-guide-list">
                  {guide.steps.map((step, index) => (
                    <div className="modal-guide-step" key={step}>
                      <span>{String(index + 1).padStart(2, "0")}</span>
                      <p>{step}</p>
                    </div>
                  ))}
                </div>

                <article className="modal-meta-card">
                  <div className="modal-meta-row">
                    <span>回调页</span>
                    <strong>{callbackUrl}</strong>
                  </div>
                  <div className="modal-meta-row">
                    <span>账号池</span>
                    <strong>{accountCount} 个账号</strong>
                  </div>
                  <div className="modal-meta-row">
                    <span>租户策略</span>
                    <strong>{statusLabel}</strong>
                  </div>
                  {loginId ? (
                    <div className="modal-meta-row">
                      <span>当前会话</span>
                      <strong>{loginId}</strong>
                    </div>
                  ) : null}
                </article>
              </aside>

              <div className="modal-content">
                {activeTab === "login" ? (
                  <div className="modal-body">
                    <div className="inline-note">
                      <strong>无需先创建租户</strong>
                      <span>
                        {tenantCount === 0
                          ? "首次授权成功后，系统会自动补一个默认租户。"
                          : "当前环境已有租户，账号会直接写入当前控制面。"}
                      </span>
                    </div>

                    <div className="modal-grid">
                      <label className="modal-field">
                        <span>账号别名</span>
                        <input
                          onChange={(event) => setLabel(event.target.value)}
                          placeholder="可留空，默认读取 OpenAI 账号信息"
                          type="text"
                          value={label}
                        />
                      </label>
                      <label className="modal-field">
                        <span>备注</span>
                        <input
                          onChange={(event) => setNote(event.target.value)}
                          placeholder="例如：主账号 / 团队工作区"
                          type="text"
                          value={note}
                        />
                      </label>
                      <label className="modal-field modal-span-2">
                        <span>回调地址</span>
                        <input readOnly type="text" value={callbackUrl} />
                      </label>
                    </div>

                    <div className="callout-card modal-callout">
                      打开的是 OpenAI 官方授权页。成功后会自动返回当前回调页，并把结果通知主页面。
                    </div>

                    <div className="modal-actions">
                      <button
                        className="button primary"
                        disabled={isPending}
                        onClick={handleStartLogin}
                        type="button"
                      >
                        <span>{loginId ? "重新打开授权页" : "登录 OpenAI"}</span>
                      </button>
                      <a
                        className="button ghost"
                        href={loginUrl || callbackUrl}
                        rel="noreferrer"
                        target="_blank"
                      >
                        <span>{loginUrl ? "打开当前授权页" : "打开回调页"}</span>
                      </a>
                    </div>
                  </div>
                ) : null}

                {activeTab === "callback" ? (
                  <div className="modal-body">
                    <div className="inline-note">
                      <strong>完整地址里必须带 state 和 code</strong>
                      <span>
                        如果浏览器已经停在回调页，直接把地址栏里的完整内容整段粘贴进来即可。
                      </span>
                    </div>

                    <label className="modal-field">
                      <span>登录成功后的完整地址</span>
                      <textarea
                        onChange={(event) => setCallbackValue(event.target.value)}
                        placeholder="粘贴包含 state 和 code 的完整 URL"
                        rows={8}
                        value={callbackValue}
                      />
                    </label>

                    <div className="modal-actions">
                      <button
                        className="button primary"
                        disabled={isPending}
                        onClick={handleManualCallback}
                        type="button"
                      >
                        <span>解析并导入</span>
                      </button>
                      <a
                        className="button ghost"
                        href={callbackUrl}
                        rel="noreferrer"
                        target="_blank"
                      >
                        <span>打开回调页</span>
                      </a>
                    </div>
                  </div>
                ) : null}

                {activeTab === "import" ? (
                  <div className="modal-body">
                    <div className="inline-note">
                      <strong>支持 JSON、TXT、LOG</strong>
                      <span>适合把已有账号文件一次拖进来，由系统自动解析并导入。</span>
                    </div>

                    <label className="modal-field">
                      <span>账号文件</span>
                      <input
                        accept=".json,.txt,.log"
                        multiple
                        onChange={(event) =>
                          setSelectedFiles(Array.from(event.target.files ?? []))
                        }
                        type="file"
                      />
                    </label>

                    {selectedFiles.length > 0 ? (
                      <div className="upload-list">
                        {selectedFiles.map((file) => (
                          <div className="upload-item" key={`${file.name}-${file.size}`}>
                            <strong>{file.name}</strong>
                            <span>{Math.max(1, Math.round(file.size / 1024))} KB</span>
                          </div>
                        ))}
                      </div>
                    ) : (
                      <div className="upload-empty">选择账号文件后即可一键导入。</div>
                    )}

                    <div className="modal-actions">
                      <button
                        className="button primary"
                        disabled={isPending || selectedFiles.length === 0}
                        onClick={handleImportFiles}
                        type="button"
                      >
                        <span>一键导入账号</span>
                      </button>
                    </div>
                  </div>
                ) : null}
              </div>
            </div>
          </section>
        </div>
      ) : null}
    </>
  );
}
