import Link from "next/link";
import { headers } from "next/headers";
import { BellRing, CheckCircle2, Link2, ShieldAlert } from "lucide-react";
import { parseOpenAiCallbackAction } from "@/app/actions";
import { OAuthCallbackSignal } from "@/components/account/OAuthCallbackSignal";
import { completeOpenAiLogin, getOpenAiLoginStatus } from "@/lib/dashboard";
import { getOpenAiCallbackPublicUrl } from "@/lib/openai-oauth";

type SearchParams =
  | Promise<Record<string, string | string[] | undefined>>
  | Record<string, string | string[] | undefined>
  | undefined;

function firstValue(value: string | string[] | undefined) {
  return Array.isArray(value) ? value[0] : value;
}

export default async function OAuthCallbackPage({
  searchParams
}: {
  searchParams?: SearchParams;
}) {
  const params = ((await searchParams) ??
    {}) as Record<string, string | string[] | undefined>;
  const state = firstValue(params.state) ?? "";
  const code = firstValue(params.code) ?? "";
  const presetNoticeMessage = firstValue(params.noticeMessage) ?? "";
  const presetNoticeTone =
    firstValue(params.noticeTone) === "error" ? "error" : "ok";

  const requestHeaders = await headers();
  const acceptLanguage = requestHeaders.get("accept-language") ?? "";
  const language = acceptLanguage.toLowerCase().includes("zh") ? "zh" : "en";
  const t =
    language === "zh"
      ? {
          kicker: "OAuth 回调",
          title: "OpenAI 授权回调",
          intro:
            "如果这是从授权页返回，系统会优先自动导入；如果失败，你仍然可以手动粘贴最终地址完成解析。",
          ready: "回调就绪",
          attention: "需要处理",
          waiting: "等待回调...",
          current: "当前回调",
          callbackUrl: "回调地址",
          browserUrl: "当前浏览器地址",
          manual: "手动解析",
          finalUrl: "浏览器最终地址",
          placeholder: "粘贴登录成功后的完整 URL",
          submit: "解析并导入",
          back: "返回控制台",
          success: (label: string) => `授权完成，已导入账号 ${label}。`,
          successPlain: "授权完成，账号已导入控制面。",
          error: "授权回调解析失败。"
        }
      : {
          kicker: "OAuth Callback",
          title: "OpenAI OAuth Callback",
          intro:
            "If this page was reached from the authorization flow, the system will try to import automatically first. If it fails, you can still paste the final URL to finish the callback parsing manually.",
          ready: "Callback Ready",
          attention: "Needs Attention",
          waiting: "Waiting for callback...",
          current: "Current Callback",
          callbackUrl: "Callback URL",
          browserUrl: "Current Browser URL",
          manual: "Manual Parse",
          finalUrl: "Final Browser URL",
          placeholder: "Paste the final successful callback URL",
          submit: "Parse and Import",
          back: "Back to Dashboard",
          success: (label: string) => `Authorization complete. Imported account ${label}.`,
          successPlain: "Authorization complete. Account imported into the control plane.",
          error: "Failed to parse the authorization callback."
        };
  const forwardedProto = requestHeaders.get("x-forwarded-proto");
  const forwardedHost =
    requestHeaders.get("x-forwarded-host") ?? requestHeaders.get("host");
  const webOrigin = forwardedHost
    ? `${forwardedProto ?? "http"}://${forwardedHost}`
    : "http://127.0.0.1:3000";
  const redirectUri = getOpenAiCallbackPublicUrl();
  const callbackUrl =
    state && code
      ? `${redirectUri}?state=${encodeURIComponent(state)}&code=${encodeURIComponent(code)}`
      : "";

  let noticeTone: "ok" | "error" = presetNoticeTone;
  let noticeMessage = presetNoticeMessage;
  let importedLabel = "";

  if (!noticeMessage && state && code) {
    try {
      await completeOpenAiLogin({
        state,
        code,
        redirectUri
      });
      const session = await getOpenAiLoginStatus(state).catch(() => null);
      importedLabel = session?.importedAccountLabel ?? "";
      noticeTone = "ok";
      noticeMessage = importedLabel ? t.success(importedLabel) : t.successPlain;
    } catch (error) {
      noticeTone = "error";
      noticeMessage = error instanceof Error ? error.message : t.error;
    }
  }

  const Icon = noticeTone === "ok" ? CheckCircle2 : ShieldAlert;

  return (
    <main className="min-h-screen bg-[#f5f5f7] px-4 py-10 text-zinc-900">
      {noticeMessage ? (
        <OAuthCallbackSignal
          importedLabel={importedLabel}
          message={noticeMessage}
          tone={noticeTone}
        />
      ) : null}

      <div className="mx-auto max-w-3xl space-y-6">
        <section className="rounded-[32px] border border-white/70 bg-white/85 p-8 shadow-panel backdrop-blur-xl">
          <header className="flex flex-col gap-4 md:flex-row md:items-start md:justify-between">
            <div>
              <p className="text-xs uppercase tracking-[0.24em] text-zinc-400">
                {t.kicker}
              </p>
              <h1 className="mt-2 text-2xl font-semibold tracking-tight text-zinc-900">
                {t.title}
              </h1>
              <p className="mt-3 max-w-2xl text-sm leading-6 text-zinc-500">
                {t.intro}
              </p>
            </div>

            <span
              className={`inline-flex items-center gap-2 rounded-full px-4 py-2 text-xs font-medium ${
                noticeTone === "ok"
                  ? "bg-emerald-50 text-emerald-700"
                  : "bg-amber-50 text-amber-700"
              }`}
            >
              <BellRing size={14} strokeWidth={1.5} />
              {noticeTone === "ok" ? t.ready : t.attention}
            </span>
          </header>

          <div
            className={`mt-6 flex items-start gap-3 rounded-[24px] p-4 ${
              noticeTone === "ok"
                ? "bg-emerald-50 text-emerald-700"
                : "bg-amber-50 text-amber-700"
            }`}
          >
            <span className="mt-0.5 flex h-9 w-9 items-center justify-center rounded-2xl bg-white/80">
              <Icon size={18} strokeWidth={1.5} />
            </span>
            <div>
              <p className="text-sm font-medium">{noticeMessage || t.waiting}</p>
            </div>
          </div>

          <div className="mt-6 grid gap-4 md:grid-cols-2">
            <article className="rounded-[24px] bg-zinc-50 p-5">
              <div className="mb-4 flex items-center gap-2 text-sm font-medium text-zinc-900">
                <Link2 size={16} strokeWidth={1.5} />
                {t.current}
              </div>
              <div className="space-y-4">
                <label className="block">
                  <span className="mb-2 block text-xs text-zinc-500">{t.callbackUrl}</span>
                  <input
                    className="w-full rounded-2xl border border-zinc-200 bg-white px-4 py-3 text-sm text-zinc-700 outline-none"
                    defaultValue={redirectUri}
                    readOnly
                    type="text"
                  />
                </label>
                <label className="block">
                  <span className="mb-2 block text-xs text-zinc-500">{t.browserUrl}</span>
                  <textarea
                    className="min-h-[120px] w-full rounded-2xl border border-zinc-200 bg-white px-4 py-3 text-sm text-zinc-700 outline-none"
                    defaultValue={callbackUrl}
                    readOnly
                  />
                </label>
              </div>
            </article>

            <form
              action={parseOpenAiCallbackAction}
              className="rounded-[24px] bg-zinc-50 p-5"
            >
              <input name="returnTo" type="hidden" value="callback" />
              <div className="mb-4 flex items-center gap-2 text-sm font-medium text-zinc-900">
                <ShieldAlert size={16} strokeWidth={1.5} />
                {t.manual}
              </div>
              <label className="block">
                <span className="mb-2 block text-xs text-zinc-500">{t.finalUrl}</span>
                <textarea
                  className="min-h-[168px] w-full rounded-2xl border border-zinc-200 bg-white px-4 py-3 text-sm text-zinc-700 outline-none"
                  defaultValue={callbackUrl}
                  name="callbackUrl"
                  placeholder={t.placeholder}
                />
              </label>

              <div className="mt-4 flex flex-wrap gap-3">
                <button
                  className="inline-flex items-center rounded-2xl bg-zinc-900 px-4 py-3 text-sm font-medium text-white transition-all duration-200 hover:opacity-90"
                  type="submit"
                >
                  {t.submit}
                </button>
                <Link
                  className="inline-flex items-center rounded-2xl bg-white px-4 py-3 text-sm font-medium text-zinc-700 shadow-soft transition-all duration-200 hover:bg-zinc-100"
                  href="/"
                >
                  {t.back}
                </Link>
              </div>
            </form>
          </div>
        </section>
      </div>
    </main>
  );
}
