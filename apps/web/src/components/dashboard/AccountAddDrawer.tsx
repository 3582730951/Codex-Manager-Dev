"use client";

import { useEffect, useRef, useState } from "react";
import { useRouter } from "next/navigation";
import {
  ArrowUpRight,
  ChevronRight,
  CheckCircle2,
  Copy,
  FileUp,
  Link2,
  ShieldAlert,
  Sparkles,
  Upload,
  X
} from "lucide-react";

type Language = "zh" | "en";
type ThemeMode = "dark" | "light";

type AccountAddDrawerProps = {
  open: boolean;
  onClose: () => void;
  language: Language;
  callbackUrl: string;
  accountCount: number;
  tenantCount: number;
  theme: ThemeMode;
};

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

type StatusTone = "neutral" | "ok" | "error";

function cx(...values: Array<string | false | null | undefined>) {
  return values.filter(Boolean).join(" ");
}

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

function copyFor(language: Language) {
  return language === "zh"
    ? {
        title: "添加账号",
        subtitle: "更接近 Codex 官方登录流。",
        status: "接入状态",
        firstTenant: "首次授权会自动创建默认租户",
        poolStatus: (accountCount: number, tenantCount: number) =>
          `当前 ${tenantCount} 个租户，${accountCount} 个账号`,
        sessionLive: "授权会话进行中",
        sessionIdle: "等待发起连接",
        completed: "已完成",
        needsAction: "需要处理",
        preparing: "准备中",
        defaultStatus:
          "生成完整授权链接后，优先在浏览器完成登录，再按需粘贴回调补导入。",
        generatingFirst: "正在生成授权链接，首次成功后会自动创建默认租户。",
        generating: "正在生成 OpenAI 授权链接...",
        connectionTitle: "授权连接",
        connectionDesc: "主流程",
        connect: "连接 OpenAI",
        reopen: "重新打开授权页",
        copyLink: "复制登录链接",
        generateAndCopy: "生成并复制链接",
        openCurrent: "打开当前授权页",
        openCallback: "打开回调页",
        authUrl: "完整授权链接",
        authUrlPlaceholder: "点击上方按钮后，这里会显示完整 authUrl。",
        callbackUrl: "回调地址",
        currentSession: "当前会话",
        advanced: "高级项",
        label: "账号别名",
        labelPlaceholder: "默认读取 OpenAI 账号信息",
        note: "备注",
        notePlaceholder: "例如：主账号 / 团队工作区",
        manualTitle: "回调补导入",
        manualDesc: "自动回调失败时使用",
        manualField: "登录成功后的完整地址",
        manualPlaceholder: "粘贴包含 state 和 code 的完整 URL",
        parseAndImport: "解析并导入",
        fileTitle: "文件导入",
        fileDesc: "批量上号入口",
        fileInput: "账号文件",
        noFiles: "选择账号文件后即可导入。",
        importFiles: "导入账号文件",
        copied:
          "授权链接已复制。请粘贴到你的浏览器地址栏打开。",
        popupBlocked: "浏览器拦截了新标签页。请改用“复制登录链接”。",
        authUrlReady:
          "完整授权链接已生成。你可以直接复制到常用浏览器打开。",
        opened:
          "授权页已打开；系统会轮询登录状态，失败时可以直接使用回调补导入。",
        openedManual:
          "授权页已打开；如果自动回调失败，再使用下面的回调补导入。",
        startFailed: "OpenAI 授权启动失败。",
        loginStatusFailed: "读取登录状态失败。",
        genericSuccess: "OpenAI 账号已导入控制面。",
        importedAccount: (label: string) => `已导入账号 ${label}`,
        failedRetry: "授权失败，请重试或改用回调补导入。",
        callbackRequired: "请先粘贴登录成功后的完整回调地址。",
        callbackMissing:
          "回调地址里缺少 state 或 code。请粘贴浏览器最终停留的完整地址。",
        parsing: "正在解析回调并导入账号...",
        parseFailed: "回调解析失败。",
        filesRequired: "请先选择账号文件。",
        importing: "正在导入账号文件...",
        importFailed: "账号导入失败。",
        importSummary: (created: number, failed: number) =>
          failed > 0
            ? `导入完成，成功 ${created} 条，失败 ${failed} 条。`
            : `导入完成，已新增 ${created} 个账号。`,
        close: "关闭",
        kb: "KB",
        optional: "可选",
        callbackHint: "应与官方 Codex 登录回调保持一致",
        manualHint: "粘贴浏览器最终停留地址",
        fileHint: "支持 JSON / TXT / LOG"
      }
    : {
        title: "Add Account",
        subtitle: "Closer to the Codex-native login flow.",
        status: "Intake Status",
        firstTenant: "The first authorization will create a default tenant automatically",
        poolStatus: (accountCount: number, tenantCount: number) =>
          `${tenantCount} tenants, ${accountCount} accounts`,
        sessionLive: "Authorization session active",
        sessionIdle: "Waiting to connect",
        completed: "Completed",
        needsAction: "Needs Action",
        preparing: "Preparing",
        defaultStatus:
          "Generate the full authorization URL first, finish login in the browser, then paste the callback only if needed.",
        generating:
          "Generating the OpenAI authorization URL...",
        generatingFirst:
          "Generating the authorization URL. The first success will create the default tenant automatically.",
        connectionTitle: "Authorization",
        connectionDesc: "Primary flow",
        connect: "Connect OpenAI",
        reopen: "Reopen Auth Page",
        copyLink: "Copy Login Link",
        generateAndCopy: "Generate and Copy",
        openCurrent: "Open Current Auth Page",
        openCallback: "Open Callback Page",
        authUrl: "Full Authorization URL",
        authUrlPlaceholder: "The full authUrl will appear here after you start the flow.",
        callbackUrl: "Callback URL",
        currentSession: "Current Session",
        advanced: "Advanced",
        label: "Account label",
        labelPlaceholder: "Defaults to the OpenAI account profile",
        note: "Note",
        notePlaceholder: "For example: Primary account / Team workspace",
        manualTitle: "Callback Fallback",
        manualDesc: "Use only when auto-return fails",
        manualField: "Final successful browser URL",
        manualPlaceholder: "Paste the full URL containing state and code",
        parseAndImport: "Parse and Import",
        fileTitle: "File Import",
        fileDesc: "Bulk intake path",
        fileInput: "Account Files",
        noFiles: "Select account files to import.",
        importFiles: "Import Account Files",
        copied: "Authorization URL copied. Paste it into your browser to continue.",
        popupBlocked: "The browser blocked the new tab. Use “Copy Login Link” instead.",
        authUrlReady:
          "The full authorization URL is ready. You can copy it into your preferred browser.",
        opened:
          "The authorization page is open. The system will keep polling; use callback fallback if needed.",
        openedManual:
          "The authorization page is open. Use the callback fallback below if auto-return fails.",
        startFailed: "Failed to start the OpenAI authorization flow.",
        loginStatusFailed: "Failed to read the login status.",
        genericSuccess: "The OpenAI account was imported into the control plane.",
        importedAccount: (label: string) => `Imported account ${label}.`,
        failedRetry: "Authorization failed. Retry or use the callback fallback.",
        callbackRequired: "Paste the full callback URL first.",
        callbackMissing:
          "The callback URL is missing state or code. Paste the final full browser URL.",
        parsing: "Parsing the callback and importing the account...",
        parseFailed: "Failed to parse the callback.",
        filesRequired: "Select account files first.",
        importing: "Importing account files...",
        importFailed: "Failed to import the account files.",
        importSummary: (created: number, failed: number) =>
          failed > 0
            ? `Import completed. ${created} created, ${failed} failed.`
            : `Import completed. Added ${created} accounts.`,
        close: "Close",
        kb: "KB",
        optional: "Optional",
        callbackHint: "Keep this aligned with the Codex callback target",
        manualHint: "Paste the final browser URL",
        fileHint: "Supports JSON / TXT / LOG"
      };
}

function normalizeCallbackUrl(
  rawValue: string,
  callbackUrl: string,
  fallbackRequired: string,
  fallbackMissing: string
) {
  const value = rawValue.trim();
  if (!value) {
    throw new Error(fallbackRequired);
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

  throw new Error(fallbackMissing);
}

export function AccountAddDrawer({
  open,
  onClose,
  language,
  callbackUrl,
  accountCount,
  tenantCount,
  theme
}: AccountAddDrawerProps) {
  const router = useRouter();
  const t = copyFor(language);
  const isDark = theme === "dark";
  const numberLocale = language === "zh" ? "zh-CN" : "en-US";
  const popupRef = useRef<Window | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const [label, setLabel] = useState("");
  const [note, setNote] = useState("");
  const [callbackValue, setCallbackValue] = useState("");
  const [loginUrl, setLoginUrl] = useState("");
  const [loginId, setLoginId] = useState("");
  const [copyReady, setCopyReady] = useState(false);
  const [selectedFiles, setSelectedFiles] = useState<File[]>([]);
  const [busy, setBusy] = useState(false);
  const [statusTone, setStatusTone] = useState<StatusTone>("neutral");
  const [statusMessage, setStatusMessage] = useState(t.defaultStatus);

  useEffect(() => {
    setStatusMessage((current) =>
      current.trim() ? current : t.defaultStatus
    );
  }, [t.defaultStatus]);

  useEffect(() => {
    const messages = copyFor(language);

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
        clearLoginSession();
        setBusy(false);
        setStatusTone("ok");
        setStatusMessage(
          payload.importedLabel
            ? messages.importedAccount(payload.importedLabel)
            : payload.message || messages.genericSuccess
        );
        setCallbackValue("");
        setSelectedFiles([]);
        if (fileInputRef.current) {
          fileInputRef.current.value = "";
        }
        router.refresh();
      }
      if (payload.type === "codex-manager:login-error") {
        setStatusTone("error");
        setStatusMessage(payload.message || messages.parseFailed);
      }
    }

    window.addEventListener("message", handleMessage);
    return () => window.removeEventListener("message", handleMessage);
  }, [language, router]);

  useEffect(() => {
    if (!loginId) {
      return undefined;
    }

    const timer = window.setInterval(async () => {
      try {
        const response = await fetch(`/api/account-intake/login/${loginId}`, {
          cache: "no-store"
        });
        const session = await readJson<LoginSession>(
          response,
          t.loginStatusFailed
        );
        const status = String(session.status || "").trim().toLowerCase();

        if (status === "success") {
          finishSuccess(
            session.importedAccountLabel
              ? t.importedAccount(session.importedAccountLabel)
              : t.genericSuccess
          );
        }

        if (status === "failed") {
          setStatusTone("error");
          setStatusMessage(session.error?.trim() || t.failedRetry);
          clearLoginSession();
        }
      } catch {
        // ignore transient polling failures
      }
    }, 1500);

    return () => window.clearInterval(timer);
  }, [loginId, t.failedRetry, t.genericSuccess, t.importedAccount, t.loginStatusFailed]);

  function clearLoginSession() {
    setLoginId("");
    if (popupRef.current && !popupRef.current.closed) {
      popupRef.current.close();
    }
    popupRef.current = null;
  }

  function finishSuccess(message: string) {
    clearLoginSession();
    setBusy(false);
    setStatusTone("ok");
    setStatusMessage(message);
    setCallbackValue("");
    setSelectedFiles([]);
    if (fileInputRef.current) {
      fileInputRef.current.value = "";
    }
    router.refresh();
  }

  async function createLoginSession(options?: {
    openTab?: boolean;
    manualOnly?: boolean;
  }) {
    setBusy(true);
    setStatusTone("neutral");
    setStatusMessage(tenantCount === 0 ? t.generatingFirst : t.generating);

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

      const result = await readJson<LoginStartResult>(response, t.startFailed);
      setLoginId(result.loginId);
      setLoginUrl(result.authUrl);
      setCopyReady(true);

      if (options?.openTab === false) {
        setStatusMessage(t.authUrlReady);
        return result.authUrl;
      }

      const popup = window.open(result.authUrl, "_blank");
      if (!popup) {
        setStatusTone("error");
        setStatusMessage(t.popupBlocked);
        return result.authUrl;
      }

      popupRef.current = popup;
      popup.focus();
      setStatusMessage(options?.manualOnly ? t.openedManual : t.opened);
      return result.authUrl;
    } finally {
      setBusy(false);
    }
  }

  async function handleStartLogin() {
    try {
      await createLoginSession();
    } catch (error) {
      setStatusTone("error");
      setStatusMessage(error instanceof Error ? error.message : t.startFailed);
    }
  }

  async function handleCopyLoginUrl() {
    try {
      const authUrl =
        loginUrl.trim() ||
        (await createLoginSession({ openTab: false, manualOnly: true }));
      if (!authUrl) {
        throw new Error(t.startFailed);
      }
      await navigator.clipboard.writeText(authUrl);
      setCopyReady(true);
      setStatusTone("ok");
      setStatusMessage(t.copied);
    } catch (error) {
      setStatusTone("error");
      setStatusMessage(error instanceof Error ? error.message : t.startFailed);
    }
  }

  async function handleManualCallback() {
    try {
      setBusy(true);
      const normalizedCallbackUrl = normalizeCallbackUrl(
        callbackValue,
        callbackUrl,
        t.callbackRequired,
        t.callbackMissing
      );
      setStatusTone("neutral");
      setStatusMessage(t.parsing);

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
      }>(response, t.parseFailed);

      finishSuccess(
        payload.session?.importedAccountLabel
          ? t.importedAccount(payload.session.importedAccountLabel)
          : t.genericSuccess
      );
    } catch (error) {
      setBusy(false);
      setStatusTone("error");
      setStatusMessage(error instanceof Error ? error.message : t.parseFailed);
    }
  }

  async function handleImportFiles() {
    if (selectedFiles.length === 0) {
      setStatusTone("error");
      setStatusMessage(t.filesRequired);
      return;
    }

    try {
      setBusy(true);
      setStatusTone("neutral");
      setStatusMessage(t.importing);

      const contents = await Promise.all(selectedFiles.map((file) => file.text()));
      const response = await fetch("/api/account-intake/import", {
        method: "POST",
        headers: {
          "content-type": "application/json"
        },
        body: JSON.stringify({ contents })
      });

      const result = await readJson<ImportResult>(response, t.importFailed);
      finishSuccess(t.importSummary(result.created, result.failed));
    } catch (error) {
      setBusy(false);
      setStatusTone("error");
      setStatusMessage(error instanceof Error ? error.message : t.importFailed);
    }
  }

  const statusLabel =
    tenantCount === 0 ? t.firstTenant : t.poolStatus(accountCount, tenantCount);

  return (
    <div
      className={cx(
        "fixed inset-0 z-[60] transition-all duration-200",
        open ? "pointer-events-auto" : "pointer-events-none"
      )}
    >
      <div
        className={cx(
          "absolute inset-0 transition-opacity duration-200",
          isDark ? "bg-[#05070b]/72" : "bg-zinc-900/20",
          open ? "opacity-100" : "opacity-0"
        )}
        onClick={onClose}
      />

      <aside
        className={cx(
          "absolute inset-y-4 right-4 w-full max-w-[560px] overflow-hidden rounded-[32px] border shadow-panel backdrop-blur-xl transition-all duration-200",
          isDark
            ? "border-white/10 bg-[#11141b]/95"
            : "border-white/70 bg-white/95",
          open ? "translate-x-0 opacity-100" : "translate-x-10 opacity-0"
        )}
      >
        <div className="flex h-full flex-col">
          <header
            className={cx(
              "flex items-start justify-between gap-4 border-b px-6 py-5",
              isDark ? "border-white/10" : "border-zinc-200/80"
            )}
          >
            <div>
              <p className={cx("text-xs uppercase tracking-[0.22em]", isDark ? "text-zinc-500" : "text-zinc-400")}>
                {t.status}
              </p>
              <h3 className={cx("mt-2 text-xl font-semibold tracking-tight", isDark ? "text-zinc-50" : "text-zinc-900")}>
                {t.title}
              </h3>
              <p className={cx("mt-2 max-w-md text-sm", isDark ? "text-zinc-400" : "text-zinc-500")}>
                {t.subtitle}
              </p>
            </div>
            <button
              className={cx(
                "flex h-10 w-10 items-center justify-center rounded-2xl transition-all duration-200",
                isDark
                  ? "bg-white/[0.06] text-zinc-400 hover:bg-white/[0.1]"
                  : "bg-zinc-100 text-zinc-500 hover:bg-zinc-200"
              )}
              onClick={onClose}
              type="button"
            >
              <X size={18} strokeWidth={1.5} />
            </button>
          </header>

          <div className="flex-1 space-y-5 overflow-y-auto px-6 py-5">
            <div
              className={cx(
                "flex items-center justify-between gap-4 rounded-[24px] px-4 py-3",
                isDark ? "bg-[#0c0f15]" : "bg-zinc-50"
              )}
            >
              <div>
                <p className={cx("text-sm font-medium", isDark ? "text-zinc-100" : "text-zinc-900")}>{statusLabel}</p>
                <p className={cx("mt-1 text-xs", isDark ? "text-zinc-500" : "text-zinc-500")}>{statusMessage}</p>
              </div>
              <span
                className={cx(
                  "rounded-full px-3 py-1.5 text-xs font-medium",
                  statusTone === "ok"
                    ? "bg-emerald-100 text-emerald-700"
                    : statusTone === "error"
                      ? "bg-rose-100 text-rose-700"
                      : "bg-zinc-200 text-zinc-600"
                )}
              >
                {statusTone === "ok"
                  ? t.completed
                  : statusTone === "error"
                    ? t.needsAction
                    : t.preparing}
              </span>
            </div>

            <section
              className={cx(
                "space-y-4 rounded-[28px] p-4",
                isDark ? "bg-[#0c0f15]" : "bg-zinc-50"
              )}
            >
              <div className="flex items-start gap-3">
                <span
                  className={cx(
                    "mt-0.5 flex h-10 w-10 items-center justify-center rounded-2xl shadow-soft",
                    isDark ? "bg-white/[0.06] text-sky-200" : "bg-white text-sky-600"
                  )}
                >
                  <Sparkles size={18} strokeWidth={1.5} />
                </span>
                <div>
                  <h4 className={cx("text-sm font-medium", isDark ? "text-zinc-100" : "text-zinc-900")}>
                    {t.connectionTitle}
                  </h4>
                  <p className={cx("mt-1 text-xs", isDark ? "text-zinc-500" : "text-zinc-500")}>
                    {t.connectionDesc}
                  </p>
                </div>
              </div>

              <div className={cx("grid gap-3", loginUrl ? "sm:grid-cols-3" : "sm:grid-cols-2")}>
                <button
                  className="inline-flex items-center justify-center gap-2 rounded-2xl bg-zinc-900 px-4 py-3 text-sm font-medium text-white transition-all duration-200 hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-60"
                  disabled={busy}
                  onClick={handleStartLogin}
                  type="button"
                >
                  <Sparkles size={16} strokeWidth={1.5} />
                  {loginId ? t.reopen : t.connect}
                </button>
                <button
                  className="inline-flex items-center justify-center gap-2 rounded-2xl bg-white px-4 py-3 text-sm font-medium text-zinc-700 shadow-soft transition-all duration-200 hover:bg-zinc-100 disabled:cursor-not-allowed disabled:opacity-60"
                  disabled={busy}
                  onClick={handleCopyLoginUrl}
                  type="button"
                >
                  <Copy size={16} strokeWidth={1.5} />
                  {copyReady ? t.copyLink : t.generateAndCopy}
                </button>
                {loginUrl ? (
                  <button
                    className="inline-flex items-center justify-center gap-2 rounded-2xl bg-white px-4 py-3 text-sm font-medium text-zinc-700 shadow-soft transition-all duration-200 hover:bg-zinc-100"
                    onClick={() => window.open(loginUrl, "_blank")}
                    type="button"
                  >
                    <ArrowUpRight size={16} strokeWidth={1.5} />
                    {t.openCurrent}
                  </button>
                ) : null}
              </div>

              <label className="block">
                <span className="mb-2 flex items-center gap-2 text-xs text-zinc-500">
                  <Link2 size={14} strokeWidth={1.5} />
                  {t.authUrl}
                </span>
                <textarea
                  className="min-h-[132px] w-full rounded-[24px] border border-zinc-200 bg-white px-4 py-3 text-sm text-zinc-700 outline-none transition-all duration-200 focus:border-sky-300"
                  readOnly
                  rows={5}
                  value={loginUrl || t.authUrlPlaceholder}
                />
              </label>

              <div className="rounded-[24px] border border-zinc-200 bg-white px-4 py-3">
                <p className="text-[11px] uppercase tracking-[0.18em] text-zinc-400">
                  {t.callbackUrl}
                </p>
                <p className="mt-2 break-all text-sm text-zinc-700">{callbackUrl}</p>
                <p className="mt-1 text-xs text-zinc-400">{t.callbackHint}</p>
              </div>

              {loginId ? (
                <div className="inline-flex items-center gap-2 rounded-full bg-sky-50 px-3 py-2 text-xs font-medium text-sky-700">
                  <CheckCircle2 size={14} strokeWidth={1.5} />
                  {t.currentSession}: {loginId}
                </div>
              ) : null}

              <details className="group rounded-[24px] border border-zinc-200 bg-white px-4 py-3">
                <summary className="flex cursor-pointer list-none items-center justify-between gap-3">
                  <div>
                    <p className="text-sm font-medium text-zinc-900">{t.advanced}</p>
                    <p className="mt-1 text-xs text-zinc-400">{t.optional}</p>
                  </div>
                  <ChevronRight
                    className="text-zinc-400 transition-transform duration-200 group-open:rotate-90"
                    size={16}
                    strokeWidth={1.5}
                  />
                </summary>
                <div className="mt-4 grid gap-3 sm:grid-cols-2">
                  <label className="block">
                    <span className="mb-2 block text-xs text-zinc-500">{t.label}</span>
                    <input
                      className="w-full rounded-2xl border border-zinc-200 bg-zinc-50 px-4 py-3 text-sm text-zinc-700 outline-none transition-all duration-200 focus:border-sky-300"
                      onChange={(event) => setLabel(event.target.value)}
                      placeholder={t.labelPlaceholder}
                      type="text"
                      value={label}
                    />
                  </label>
                  <label className="block">
                    <span className="mb-2 block text-xs text-zinc-500">{t.note}</span>
                    <input
                      className="w-full rounded-2xl border border-zinc-200 bg-zinc-50 px-4 py-3 text-sm text-zinc-700 outline-none transition-all duration-200 focus:border-sky-300"
                      onChange={(event) => setNote(event.target.value)}
                      placeholder={t.notePlaceholder}
                      type="text"
                      value={note}
                    />
                  </label>
                </div>
              </details>
            </section>

            <details className="group rounded-[28px] bg-zinc-50 p-4">
              <summary className="flex cursor-pointer list-none items-center justify-between gap-4">
                <div className="flex items-start gap-3">
                  <span className="mt-0.5 flex h-10 w-10 items-center justify-center rounded-2xl bg-white text-amber-600 shadow-soft">
                    <ShieldAlert size={18} strokeWidth={1.5} />
                  </span>
                  <div>
                    <h4 className="text-sm font-medium text-zinc-900">{t.manualTitle}</h4>
                    <p className="mt-1 text-xs text-zinc-500">{t.manualDesc}</p>
                  </div>
                </div>
                <ChevronRight
                  className="text-zinc-400 transition-transform duration-200 group-open:rotate-90"
                  size={16}
                  strokeWidth={1.5}
                />
              </summary>

              <div className="mt-4 space-y-4">
                <p className="text-xs text-zinc-400">{t.manualHint}</p>
                <label className="block">
                  <span className="mb-2 block text-xs text-zinc-500">{t.manualField}</span>
                  <textarea
                    className="min-h-[148px] w-full rounded-[24px] border border-zinc-200 bg-white px-4 py-3 text-sm text-zinc-700 outline-none transition-all duration-200 focus:border-sky-300"
                    onChange={(event) => setCallbackValue(event.target.value)}
                    placeholder={t.manualPlaceholder}
                    rows={6}
                    value={callbackValue}
                  />
                </label>

                <div className="flex flex-wrap gap-3">
                  <button
                    className="inline-flex items-center justify-center gap-2 rounded-2xl bg-zinc-900 px-4 py-3 text-sm font-medium text-white transition-all duration-200 hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-60"
                    disabled={busy}
                    onClick={handleManualCallback}
                    type="button"
                  >
                    <CheckCircle2 size={16} strokeWidth={1.5} />
                    {t.parseAndImport}
                  </button>
                </div>
              </div>
            </details>

            <details className="group rounded-[28px] bg-zinc-50 p-4">
              <summary className="flex cursor-pointer list-none items-center justify-between gap-4">
                <div className="flex items-start gap-3">
                  <span className="mt-0.5 flex h-10 w-10 items-center justify-center rounded-2xl bg-white text-emerald-600 shadow-soft">
                    <FileUp size={18} strokeWidth={1.5} />
                  </span>
                  <div>
                    <h4 className="text-sm font-medium text-zinc-900">{t.fileTitle}</h4>
                    <p className="mt-1 text-xs text-zinc-500">{t.fileDesc}</p>
                  </div>
                </div>
                <ChevronRight
                  className="text-zinc-400 transition-transform duration-200 group-open:rotate-90"
                  size={16}
                  strokeWidth={1.5}
                />
              </summary>

              <div className="mt-4 space-y-4">
                <p className="text-xs text-zinc-400">{t.fileHint}</p>
                <label className="block">
                  <span className="mb-2 block text-xs text-zinc-500">{t.fileInput}</span>
                  <input
                    accept=".json,.txt,.log"
                    className="w-full rounded-2xl border border-dashed border-zinc-300 bg-white px-4 py-3 text-sm text-zinc-700 outline-none file:mr-3 file:rounded-xl file:border-0 file:bg-zinc-900 file:px-3 file:py-2 file:text-sm file:font-medium file:text-white"
                    multiple
                    onChange={(event) =>
                      setSelectedFiles(Array.from(event.target.files ?? []))
                    }
                    ref={fileInputRef}
                    type="file"
                  />
                </label>

                {selectedFiles.length > 0 ? (
                  <div className="space-y-2">
                    {selectedFiles.map((file) => (
                      <div
                        className="flex items-center justify-between rounded-2xl border border-zinc-200 bg-white px-4 py-3"
                        key={`${file.name}-${file.size}`}
                      >
                        <span className="truncate pr-3 text-sm text-zinc-700">
                          {file.name}
                        </span>
                        <span className="shrink-0 text-xs text-zinc-400">
                          {Math.max(1, Math.round(file.size / 1024)).toLocaleString(numberLocale)}{" "}
                          {t.kb}
                        </span>
                      </div>
                    ))}
                  </div>
                ) : (
                  <div className="flex min-h-[88px] items-center justify-center rounded-[24px] border border-dashed border-zinc-200 bg-white text-center text-sm text-zinc-400">
                    {t.noFiles}
                  </div>
                )}

                <button
                  className="inline-flex items-center justify-center gap-2 rounded-2xl bg-zinc-900 px-4 py-3 text-sm font-medium text-white transition-all duration-200 hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-60"
                  disabled={busy || selectedFiles.length === 0}
                  onClick={handleImportFiles}
                  type="button"
                >
                  <Upload size={16} strokeWidth={1.5} />
                  {t.importFiles}
                </button>
              </div>
            </details>
          </div>
        </div>
      </aside>
    </div>
  );
}
