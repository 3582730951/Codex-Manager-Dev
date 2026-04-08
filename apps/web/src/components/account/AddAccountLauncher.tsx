"use client";

import { useEffect, useRef, useState, useTransition } from "react";
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

  throw new Error(
    "回调地址里缺少 state 或 code。请粘贴浏览器最终停留的完整地址。",
  );
}

export function AddAccountLauncher({
  callbackUrl,
  accountCount,
  tenantCount,
}: AddAccountLauncherProps) {
  const router = useRouter();
  const [label, setLabel] = useState("");
  const [note, setNote] = useState("");
  const [callbackValue, setCallbackValue] = useState("");
  const [loginUrl, setLoginUrl] = useState("");
  const [loginId, setLoginId] = useState("");
  const [copyReady, setCopyReady] = useState(false);
  const [statusTone, setStatusTone] = useState<"neutral" | "ok" | "error">(
    "neutral",
  );
  const [statusMessage, setStatusMessage] = useState(
    "点击“连接 OpenAI”后，系统会直接生成完整授权链接，并同时支持打开、复制和后续回调补导入。",
  );
  const [selectedFiles, setSelectedFiles] = useState<File[]>([]);
  const popupRef = useRef<Window | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const [isPending, startTransition] = useTransition();

  const statusLabel =
    tenantCount === 0
      ? "首次授权会自动创建默认租户"
      : `当前 ${tenantCount} 个租户，${accountCount} 个账号`;

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
            : payload.message || "OpenAI 账号已导入控制面。",
        );
      }
      if (payload.type === "codex-manager:login-error") {
        setStatusTone("error");
        setStatusMessage(payload.message || "授权回调解析失败。");
      }
    }

    window.addEventListener("message", handleMessage);
    return () => window.removeEventListener("message", handleMessage);
  }, []);

  useEffect(() => {
    if (!loginId) {
      return undefined;
    }

    const timer = window.setInterval(async () => {
      try {
        const response = await fetch(`/api/account-intake/login/${loginId}`, {
          cache: "no-store",
        });
        const session = await readJson<LoginSession>(
          response,
          "读取登录状态失败。",
        );
        const status = String(session.status || "")
          .trim()
          .toLowerCase();

        if (status === "success") {
          finishSuccess(
            session.importedAccountLabel
              ? `已导入账号 ${session.importedAccountLabel}`
              : "OpenAI 账号已导入控制面。",
          );
        }

        if (status === "failed") {
          setStatusTone("error");
          setStatusMessage(
            session.error?.trim() || "授权失败，请重试或改用回调补导入。",
          );
          clearLoginSession();
        }
      } catch {
        // keep polling; callback page can still push a postMessage on success
      }
    }, 1500);

    return () => window.clearInterval(timer);
  }, [loginId]);

  function clearLoginSession() {
    setLoginId("");
    if (popupRef.current && !popupRef.current.closed) {
      popupRef.current.close();
    }
    popupRef.current = null;
  }

  function finishSuccess(message: string) {
    clearLoginSession();
    setStatusTone("ok");
    setStatusMessage(message);
    setCallbackValue("");
    setSelectedFiles([]);
    if (fileInputRef.current) {
      fileInputRef.current.value = "";
    }
    startTransition(() => {
      router.refresh();
    });
  }

  async function createLoginSession(options?: {
    openTab?: boolean;
    manualOnly?: boolean;
  }) {
    setStatusTone("neutral");
    setStatusMessage(
      tenantCount === 0
        ? "正在生成授权链接，首次成功后会自动创建默认租户。"
        : "正在生成 OpenAI 授权链接...",
    );

    const response = await fetch("/api/account-intake/login/start", {
      method: "POST",
      headers: {
        "content-type": "application/json",
      },
      body: JSON.stringify({
        label,
        note,
        redirectUri: callbackUrl,
      }),
    });

    const result = await readJson<LoginStartResult>(
      response,
      "OpenAI 授权地址生成失败。",
    );
    setLoginId(result.loginId);
    setLoginUrl(result.authUrl);
    setCopyReady(true);

    if (options?.openTab === false) {
      setStatusMessage("完整授权链接已生成。你可以直接复制到常用浏览器打开。");
      return result.authUrl;
    }

    const popup = window.open(result.authUrl, "_blank");
    if (!popup) {
      setStatusTone("error");
      setStatusMessage("浏览器拦截了新标签页。请改用“复制登录链接”。");
      return result.authUrl;
    }

    popupRef.current = popup;
    popup.focus();
    setStatusMessage(
      options?.manualOnly
        ? "授权页已打开；如果自动回调失败，再使用下面的回调补导入。"
        : "授权页已打开；系统会轮询登录状态，失败时可以直接使用回调补导入。",
    );
    return result.authUrl;
  }

  async function handleStartLogin() {
    try {
      await createLoginSession();
    } catch (error) {
      setStatusTone("error");
      setStatusMessage(
        error instanceof Error ? error.message : "OpenAI 授权启动失败。",
      );
    }
  }

  async function handleCopyLoginUrl() {
    try {
      const authUrl =
        loginUrl.trim() ||
        (await createLoginSession({ openTab: false, manualOnly: true }));
      if (!authUrl) {
        throw new Error("授权链接生成失败。");
      }
      await navigator.clipboard.writeText(authUrl);
      setCopyReady(true);
      setStatusTone("ok");
      setStatusMessage("授权链接已复制。请粘贴到你的浏览器地址栏打开。");
    } catch (error) {
      setStatusTone("error");
      setStatusMessage(
        error instanceof Error
          ? error.message
          : "授权链接复制失败，请手动复制。",
      );
    }
  }

  async function handleManualCallback() {
    try {
      const normalizedCallbackUrl = normalizeCallbackUrl(
        callbackValue,
        callbackUrl,
      );
      setStatusTone("neutral");
      setStatusMessage("正在解析回调并导入账号...");

      const response = await fetch("/api/account-intake/login/complete", {
        method: "POST",
        headers: {
          "content-type": "application/json",
        },
        body: JSON.stringify({
          callbackUrl: normalizedCallbackUrl,
        }),
      });

      const payload = await readJson<{
        session?: { importedAccountLabel?: string | null };
      }>(response, "回调解析失败。");

      finishSuccess(
        payload.session?.importedAccountLabel
          ? `已导入账号 ${payload.session.importedAccountLabel}`
          : "OpenAI 账号已导入控制面。",
      );
    } catch (error) {
      setStatusTone("error");
      setStatusMessage(
        error instanceof Error ? error.message : "回调解析失败。",
      );
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
      const contents = await Promise.all(
        selectedFiles.map((file) => file.text()),
      );
      const response = await fetch("/api/account-intake/import", {
        method: "POST",
        headers: {
          "content-type": "application/json",
        },
        body: JSON.stringify({ contents }),
      });

      const result = await readJson<ImportResult>(response, "账号导入失败。");
      finishSuccess(
        result.failed > 0
          ? `导入完成，成功 ${result.created} 条，失败 ${result.failed} 条。`
          : `导入完成，已新增 ${result.created} 个账号。`,
      );
    } catch (error) {
      setStatusTone("error");
      setStatusMessage(
        error instanceof Error ? error.message : "账号导入失败。",
      );
    }
  }

  return (
    <>
      <div className="intake-toolbar">
        <div className="intake-toolbar-copy">
          <strong>接入状态</strong>
          <span>{statusLabel}</span>
        </div>
        <span className="toolbar-chip">
          {loginId ? "授权会话进行中" : "等待发起连接"}
        </span>
      </div>

      <div
        className={`status-banner ${statusTone === "neutral" ? "" : statusTone}`}
      >
        <strong>
          {statusTone === "ok"
            ? "已完成"
            : statusTone === "error"
              ? "需要处理"
              : "准备中"}
        </strong>
        <p>{statusMessage}</p>
      </div>

      <div className="intake-grid">
        <article className="intake-card intake-card-primary">
          <div className="card-step">
            <span>01</span>
            <small>主操作</small>
          </div>
          <div className="card-copy">
            <strong>连接 OpenAI</strong>
            <p>
              点击后直接向后端申请完整授权链接。页面会保留
              authUrl、回调地址和当前会话状态，不再让你先进入一个工具化小弹层。
            </p>
          </div>

          <div className="button-row">
            <button
              className="button primary"
              disabled={isPending}
              onClick={handleStartLogin}
              type="button"
            >
              <span>{loginId ? "重新打开授权页" : "连接 OpenAI"}</span>
            </button>
            <button
              className="button ghost"
              disabled={isPending}
              onClick={handleCopyLoginUrl}
              type="button"
            >
              <span>{copyReady ? "复制登录链接" : "生成并复制链接"}</span>
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

          <div className="field-grid intake-fields">
            <label className="field span-2">
              <span>完整授权链接</span>
              <textarea
                readOnly
                rows={5}
                value={loginUrl || "点击上方按钮后，这里会显示完整 authUrl。"}
              />
            </label>
            <label className="field span-2">
              <span>当前回调地址</span>
              <input readOnly type="text" value={callbackUrl} />
            </label>
          </div>

          {loginId ? (
            <div className="session-inline">
              <strong>当前会话</strong>
              <code>{loginId}</code>
            </div>
          ) : null}

          <details className="detail-panel">
            <summary>高级项</summary>
            <div className="detail-body">
              <div className="field-grid compact-grid">
                <label className="field">
                  <span>账号别名，可留空</span>
                  <input
                    onChange={(event) => setLabel(event.target.value)}
                    placeholder="默认读取 OpenAI 账号信息"
                    type="text"
                    value={label}
                  />
                </label>
                <label className="field">
                  <span>备注，可留空</span>
                  <input
                    onChange={(event) => setNote(event.target.value)}
                    placeholder="例如：主账号 / 团队工作区"
                    type="text"
                    value={note}
                  />
                </label>
              </div>
            </div>
          </details>
        </article>

        <article className="intake-card">
          <div className="card-step">
            <span>02</span>
            <small>回调兜底</small>
          </div>
          <div className="card-copy">
            <strong>粘贴回调补导入</strong>
            <p>
              如果没有自动回到页面，就把浏览器最终停留的完整地址粘贴到这里，系统会自动抽取
              state 和 code。
            </p>
          </div>

          <label className="field">
            <span>登录成功后的完整地址</span>
            <textarea
              onChange={(event) => setCallbackValue(event.target.value)}
              placeholder="粘贴包含 state 和 code 的完整 URL"
              rows={9}
              value={callbackValue}
            />
          </label>

          <div className="button-row">
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
        </article>

        <article className="intake-card">
          <div className="card-step">
            <span>03</span>
            <small>批量入口</small>
          </div>
          <div className="card-copy">
            <strong>文件批量导入</strong>
            <p>
              把 JSON、TXT、LOG
              直接拖进来，系统会逐个解析并写入账号池，不再经过额外弹层。
            </p>
          </div>

          <label className="field">
            <span>账号文件</span>
            <input
              accept=".json,.txt,.log"
              multiple
              onChange={(event) =>
                setSelectedFiles(Array.from(event.target.files ?? []))
              }
              ref={fileInputRef}
              type="file"
            />
          </label>

          {selectedFiles.length > 0 ? (
            <div className="file-list">
              {selectedFiles.map((file) => (
                <div className="file-item" key={`${file.name}-${file.size}`}>
                  <strong>{file.name}</strong>
                  <span>{Math.max(1, Math.round(file.size / 1024))} KB</span>
                </div>
              ))}
            </div>
          ) : (
            <div className="upload-empty">选择账号文件后即可导入。</div>
          )}

          <div className="button-row">
            <button
              className="button primary"
              disabled={isPending || selectedFiles.length === 0}
              onClick={handleImportFiles}
              type="button"
            >
              <span>导入账号文件</span>
            </button>
          </div>
        </article>
      </div>
    </>
  );
}
