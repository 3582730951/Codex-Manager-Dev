import Link from "next/link";
import { headers } from "next/headers";
import { parseOpenAiCallbackAction } from "@/app/actions";
import { OAuthCallbackSignal } from "@/components/account/OAuthCallbackSignal";
import { getOpenAiLoginStatus, completeOpenAiLogin } from "@/lib/dashboard";
import { ParseCallbackButton } from "@/components/buttons/ParseCallbackButton";

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
  const forwardedProto = requestHeaders.get("x-forwarded-proto");
  const forwardedHost =
    requestHeaders.get("x-forwarded-host") ?? requestHeaders.get("host");
  const webOrigin =
    forwardedHost
      ? `${forwardedProto ?? "http"}://${forwardedHost}`
      : "http://127.0.0.1:3000";
  const redirectUri = `${webOrigin}/oauth/callback`;
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
      noticeMessage = importedLabel
        ? `授权完成，已导入账号 ${importedLabel}。`
        : "授权完成，账号已导入控制面。";
    } catch (error) {
      noticeTone = "error";
      noticeMessage =
        error instanceof Error ? error.message : "授权回调解析失败。";
    }
  }

  return (
    <main className="console-shell callback-shell">
      {noticeMessage ? (
        <OAuthCallbackSignal
          importedLabel={importedLabel}
          message={noticeMessage}
          tone={noticeTone}
        />
      ) : null}
      <div className="chrome-main">
        <section className="glass-card callback-stage">
          <header className="panel-head">
            <div>
              <p className="section-kicker">授权回调</p>
              <h2>OpenAI 授权回调</h2>
            </div>
            <p className="panel-note">
              如果这是从弹出的登录窗口返回，成功后会自动通知主页面；如果失败，你也可以手动粘贴完整地址。
            </p>
          </header>

          <div className={`notice ${noticeTone}`}>
            <span className="notice-mark">{noticeTone === "ok" ? "OK" : "ER"}</span>
            <p>{noticeMessage || "等待回调..."}</p>
          </div>

          <div className="callback-grid">
            <article className="form-card">
              <div className="form-head">
                <strong>当前回调</strong>
                <span>优先自动完成</span>
              </div>
              <div className="field-grid">
                <label className="field span-2">
                  <span>回调地址</span>
                  <input defaultValue={redirectUri} readOnly type="text" />
                </label>
                <label className="field span-2">
                  <span>当前浏览器地址</span>
                  <textarea defaultValue={callbackUrl} readOnly rows={4} />
                </label>
              </div>
            </article>

            <form action={parseOpenAiCallbackAction} className="form-card">
              <input name="returnTo" type="hidden" value="callback" />
              <div className="form-head">
                <strong>手动解析</strong>
                <span>粘贴包含 state 和 code 的完整地址</span>
              </div>
              <div className="field-grid">
                <label className="field span-2">
                  <span>浏览器最终地址</span>
                  <textarea
                    defaultValue={callbackUrl}
                    name="callbackUrl"
                    placeholder="粘贴登录成功后的完整 URL"
                    rows={5}
                  />
                </label>
              </div>
              <div className="form-actions dual">
                <ParseCallbackButton />
                <Link className="button ghost callback-back" href="/#login">
                  返回控制台
                </Link>
              </div>
            </form>
          </div>
        </section>
      </div>
    </main>
  );
}
