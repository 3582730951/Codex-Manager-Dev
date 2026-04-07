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

export function AddAccountLauncher({
  callbackUrl,
  accountCount,
  tenantCount
}: AddAccountLauncherProps) {
  const router = useRouter();
  const [open, setOpen] = useState(false);
  const [activeTab, setActiveTab] = useState<"login" | "callback" | "import">("login");
  const [label, setLabel] = useState("");
  const [note, setNote] = useState("");
  const [callbackValue, setCallbackValue] = useState("");
  const [loginUrl, setLoginUrl] = useState("");
  const [loginId, setLoginId] = useState("");
  const [statusTone, setStatusTone] = useState<"neutral" | "ok" | "error">("neutral");
  const [statusMessage, setStatusMessage] = useState("点击添加账号后，会直接打开 OpenAI 官方登录窗口。");
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

  useEffect(() => {
    if (!open) {
      return undefined;
    }

    const previous = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = previous;
    };
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
    setStatusMessage("点击添加账号后，会直接打开 OpenAI 官方登录窗口。");
  }

  function finishSuccess(message: string) {
    clearLoginSession();
    setStatusTone("ok");
    setStatusMessage(message);
    startTransition(() => {
      router.refresh();
    });
    closeTimerRef.current = window.setTimeout(() => {
      setOpen(false);
      resetModal();
    }, 900);
  }

  async function handleStartLogin() {
    setStatusTone("neutral");
    setStatusMessage("正在生成 OpenAI 授权窗口...");

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
        setStatusMessage("浏览器拦截了弹窗。请使用下方“重新打开授权页”。");
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
    if (!callbackValue.trim()) {
      setStatusTone("error");
      setStatusMessage("请先粘贴登录成功后的完整回调地址。");
      return;
    }

    setStatusTone("neutral");
    setStatusMessage("正在解析回调并导入账号...");
    try {
      const response = await fetch("/api/account-intake/login/complete", {
        method: "POST",
        headers: {
          "content-type": "application/json"
        },
        body: JSON.stringify({
          callbackUrl: callbackValue
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
          <strong>一键登录导入</strong>
          <p>打开 OpenAI 登录窗口，授权成功后自动回调并写入账号。</p>
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
          <strong>回调地址导入</strong>
          <p>登录完成后直接粘贴最终地址，系统会自动解析 state 与 code。</p>
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
          <strong>文件一键导入</strong>
          <p>选择 JSON 或文本账号文件，系统自动创建默认租户并完成导入。</p>
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
            <header className="modal-head">
              <div>
                <p className="section-kicker">Account Intake</p>
                <h3>添加账号</h3>
              </div>
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
                  onClick={() => setActiveTab(key as "login" | "callback" | "import")}
                  role="tab"
                  type="button"
                >
                  {title}
                </button>
              ))}
            </div>

            {activeTab === "login" ? (
              <div className="modal-body">
                <div className="modal-grid">
                  <label className="modal-field">
                    <span>账号名</span>
                    <input
                      onChange={(event) => setLabel(event.target.value)}
                      placeholder="可留空，默认会用 OpenAI 账号信息"
                      type="text"
                      value={label}
                    />
                  </label>
                  <label className="modal-field">
                    <span>备注</span>
                    <input
                      onChange={(event) => setNote(event.target.value)}
                      placeholder="例如：主号 / 团队工作区"
                      type="text"
                      value={note}
                    />
                  </label>
                  <label className="modal-field modal-span-2">
                    <span>回调地址</span>
                    <input readOnly type="text" value={callbackUrl} />
                  </label>
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
                <label className="modal-field">
                  <span>登录成功后的完整地址</span>
                  <textarea
                    onChange={(event) => setCallbackValue(event.target.value)}
                    placeholder="粘贴包含 state 和 code 的完整 URL"
                    rows={7}
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
                  <a className="button ghost" href={callbackUrl} target="_blank">
                    <span>打开回调页</span>
                  </a>
                </div>
              </div>
            ) : null}

            {activeTab === "import" ? (
              <div className="modal-body">
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
                  <div className="upload-empty">选择账号 JSON 或文本文件后即可一键导入。</div>
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
          </section>
        </div>
      ) : null}
    </>
  );
}
