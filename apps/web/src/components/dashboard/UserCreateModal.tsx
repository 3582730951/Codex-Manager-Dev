"use client";

import { useEffect, useMemo, useState } from "react";
import type { GatewayUserRole, GatewayUserView } from "@codex-manager/contracts";
import {
  CheckCircle2,
  Copy,
  KeyRound,
  Mail,
  MoonStar,
  Shield,
  Sparkles,
  UserPlus,
  Users,
  X
} from "lucide-react";

type Language = "zh" | "en";
type ThemeMode = "dark" | "light";

export type CreatedGatewayUser = {
  user: GatewayUserView;
  token: string;
};

type UserCreateModalProps = {
  open: boolean;
  onClose: () => void;
  onCreated: (created: CreatedGatewayUser) => void;
  existingEmails: string[];
  language: Language;
  availableModels: string[];
  gatewayBaseUrl: string;
  theme: ThemeMode;
};

function cx(...values: Array<string | false | null | undefined>) {
  return values.filter(Boolean).join(" ");
}

function copyFor(language: Language) {
  return language === "zh"
    ? {
        title: "添加用户",
        subtitle: "创建真实网关用户，并立即交付给下游 CLI 使用。",
        name: "姓名",
        namePlaceholder: "例如：Ava Chen",
        email: "邮箱",
        emailPlaceholder: "name@company.com",
        role: "角色",
        admin: "管理员",
        viewer: "只读",
        model: "默认模型",
        modelAuto: "沿用请求模型",
        reasoning: "默认推理强度",
        reasoningAuto: "沿用请求参数",
        overrideModel: "强制覆盖模型",
        overrideReasoning: "强制覆盖推理强度",
        cancel: "取消",
        create: "创建用户",
        creating: "创建中...",
        successTitle: "用户已创建",
        successDesc: "下面这组信息就是下游 CLI 连接网关时应使用的配置。",
        gatewayBase: "网关地址",
        gatewayKey: "网关 Key",
        shellExample: "Shell 示例",
        curlExample: "curl 示例",
        close: "关闭",
        createAnother: "继续创建",
        copy: "复制",
        copied: "已复制",
        nameRequired: "请先输入用户名。",
        emailRequired: "请先输入邮箱。",
        emailInvalid: "邮箱格式不正确。",
        emailExists: "这个邮箱已经存在。",
        createFailed: "创建用户失败。"
      }
    : {
        title: "Add User",
        subtitle: "Create a real gateway user and hand the connection details to downstream CLI immediately.",
        name: "Name",
        namePlaceholder: "For example: Ava Chen",
        email: "Email",
        emailPlaceholder: "name@company.com",
        role: "Role",
        admin: "Admin",
        viewer: "Viewer",
        model: "Default model",
        modelAuto: "Use request model",
        reasoning: "Default reasoning",
        reasoningAuto: "Use request value",
        overrideModel: "Force model override",
        overrideReasoning: "Force reasoning override",
        cancel: "Cancel",
        create: "Create User",
        creating: "Creating...",
        successTitle: "User created",
        successDesc:
          "These are the exact settings downstream CLI should use when connecting to the gateway.",
        gatewayBase: "Gateway base",
        gatewayKey: "Gateway key",
        shellExample: "Shell example",
        curlExample: "curl example",
        close: "Close",
        createAnother: "Create another",
        copy: "Copy",
        copied: "Copied",
        nameRequired: "Enter a user name first.",
        emailRequired: "Enter an email first.",
        emailInvalid: "The email format is invalid.",
        emailExists: "This email already exists.",
        createFailed: "Failed to create user."
      };
}

export function UserCreateModal({
  open,
  onClose,
  onCreated,
  existingEmails,
  language,
  availableModels,
  gatewayBaseUrl,
  theme
}: UserCreateModalProps) {
  const t = copyFor(language);
  const isDark = theme === "dark";
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [role, setRole] = useState<GatewayUserRole>("viewer");
  const [defaultModel, setDefaultModel] = useState("");
  const [reasoningEffort, setReasoningEffort] = useState("");
  const [forceModelOverride, setForceModelOverride] = useState(false);
  const [forceReasoningOverride, setForceReasoningOverride] = useState(false);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [created, setCreated] = useState<CreatedGatewayUser | null>(null);
  const [copiedField, setCopiedField] = useState("");

  const normalizedExistingEmails = useMemo(
    () => existingEmails.map((value) => value.trim().toLowerCase()),
    [existingEmails]
  );

  useEffect(() => {
    if (!open) {
      return;
    }
    setName("");
    setEmail("");
    setRole("viewer");
    setDefaultModel("");
    setReasoningEffort("");
    setForceModelOverride(false);
    setForceReasoningOverride(false);
    setError("");
    setBusy(false);
    setCreated(null);
    setCopiedField("");
  }, [open]);

  const shellSnippet = created
    ? `export OPENAI_API_BASE="${gatewayBaseUrl}"\nexport OPENAI_API_KEY="${created.token}"`
    : "";
  const curlSnippet = created
    ? `curl ${gatewayBaseUrl.replace(/\/v1$/, "")}/v1/models -H "Authorization: Bearer ${created.token}"`
    : "";

  async function copyText(value: string, field: string) {
    try {
      await navigator.clipboard.writeText(value);
      setCopiedField(field);
      window.setTimeout(() => {
        setCopiedField((current) => (current === field ? "" : current));
      }, 1200);
    } catch {
      // ignore clipboard failures
    }
  }

  async function handleSubmit() {
    const normalizedName = name.trim();
    const normalizedEmail = email.trim().toLowerCase();

    if (!normalizedName) {
      setError(t.nameRequired);
      return;
    }
    if (!normalizedEmail) {
      setError(t.emailRequired);
      return;
    }
    if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(normalizedEmail)) {
      setError(t.emailInvalid);
      return;
    }
    if (normalizedExistingEmails.includes(normalizedEmail)) {
      setError(t.emailExists);
      return;
    }

    setBusy(true);
    setError("");
    try {
      const response = await fetch("/api/dashboard/users", {
        method: "POST",
        headers: {
          "content-type": "application/json"
        },
        body: JSON.stringify({
          name: normalizedName,
          email: normalizedEmail,
          role,
          defaultModel: defaultModel || null,
          reasoningEffort: reasoningEffort || null,
          forceModelOverride,
          forceReasoningEffort: forceReasoningOverride
        })
      });
      const payload = (await response.json().catch(() => null)) as
        | CreatedGatewayUser
        | { error?: { message?: string } }
        | null;
      if (!response.ok) {
        const message =
          payload &&
          typeof payload === "object" &&
          "error" in payload
            ? payload.error?.message
            : undefined;
        throw new Error(message || t.createFailed);
      }
      const createdUser = payload as CreatedGatewayUser;
      setCreated(createdUser);
      onCreated(createdUser);
    } catch (submitError) {
      setError(
        submitError instanceof Error ? submitError.message : t.createFailed
      );
    } finally {
      setBusy(false);
    }
  }

  return (
    <div
      className={cx(
        "fixed inset-0 z-[70] transition-all duration-200",
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

      <div className="absolute inset-0 flex items-center justify-center p-4">
        <section
          className={cx(
            "w-full max-w-[620px] rounded-[32px] border p-6 shadow-panel backdrop-blur-xl transition-all duration-200",
            isDark
              ? "border-white/10 bg-[#11141b]/95 text-zinc-100"
              : "border-white/70 bg-white/95 text-zinc-900",
            open ? "translate-y-0 opacity-100" : "translate-y-4 opacity-0"
          )}
        >
          <header className="flex items-start justify-between gap-4">
            <div>
              <p
                className={cx(
                  "text-xs uppercase tracking-[0.22em]",
                  isDark ? "text-zinc-500" : "text-zinc-400"
                )}
              >
                {created ? t.successTitle : t.role}
              </p>
              <h3
                className={cx(
                  "mt-2 text-xl font-semibold tracking-tight",
                  isDark ? "text-zinc-50" : "text-zinc-900"
                )}
              >
                {created ? t.successTitle : t.title}
              </h3>
              <p
                className={cx(
                  "mt-2 text-sm leading-6",
                  isDark ? "text-zinc-400" : "text-zinc-500"
                )}
              >
                {created ? t.successDesc : t.subtitle}
              </p>
            </div>
            <button
              className={cx(
                "flex h-10 w-10 items-center justify-center rounded-2xl transition-all duration-200",
                isDark
                  ? "bg-white/6 text-zinc-400 hover:bg-white/10"
                  : "bg-zinc-100 text-zinc-500 hover:bg-zinc-200"
              )}
              onClick={onClose}
              type="button"
            >
              <X size={18} strokeWidth={1.5} />
            </button>
          </header>

          {created ? (
            <div className="mt-6 space-y-4">
              <div
                className={cx(
                  "rounded-[28px] border p-4",
                  isDark ? "border-white/10 bg-white/[0.03]" : "border-zinc-200 bg-zinc-50"
                )}
              >
                <div className="flex items-center gap-3">
                  <span
                    className={cx(
                      "flex h-11 w-11 items-center justify-center rounded-2xl",
                      isDark ? "bg-emerald-500/14 text-emerald-300" : "bg-emerald-50 text-emerald-600"
                    )}
                  >
                    <CheckCircle2 size={18} strokeWidth={1.5} />
                  </span>
                  <div>
                    <p className={cx("text-sm font-medium", isDark ? "text-zinc-100" : "text-zinc-900")}>
                      {created.user.name}
                    </p>
                    <p className={cx("mt-1 text-xs", isDark ? "text-zinc-500" : "text-zinc-500")}>
                      {created.user.email}
                    </p>
                  </div>
                </div>
              </div>

              {[
                { label: t.gatewayBase, value: gatewayBaseUrl, icon: Sparkles, field: "base" },
                { label: t.gatewayKey, value: created.token, icon: KeyRound, field: "key" }
              ].map((item) => {
                const Icon = item.icon;
                return (
                  <div
                    className={cx(
                      "rounded-[28px] border p-4",
                      isDark ? "border-white/10 bg-white/[0.03]" : "border-zinc-200 bg-zinc-50"
                    )}
                    key={item.field}
                  >
                    <div className="flex items-center justify-between gap-4">
                      <div className="min-w-0">
                        <p className={cx("flex items-center gap-2 text-xs", isDark ? "text-zinc-500" : "text-zinc-500")}>
                          <Icon size={14} strokeWidth={1.5} />
                          {item.label}
                        </p>
                        <p className={cx("mt-2 break-all text-sm font-medium", isDark ? "text-zinc-100" : "text-zinc-900")}>
                          {item.value}
                        </p>
                      </div>
                      <button
                        className={cx(
                          "inline-flex h-10 shrink-0 items-center gap-2 rounded-2xl px-3 text-sm transition-all duration-200",
                          isDark
                            ? "bg-white/6 text-zinc-300 hover:bg-white/10"
                            : "bg-white text-zinc-600 hover:bg-zinc-100"
                        )}
                        onClick={() => copyText(item.value, item.field)}
                        type="button"
                      >
                        <Copy size={14} strokeWidth={1.5} />
                        {copiedField === item.field ? t.copied : t.copy}
                      </button>
                    </div>
                  </div>
                );
              })}

              {[
                { label: t.shellExample, value: shellSnippet, field: "shell" },
                { label: t.curlExample, value: curlSnippet, field: "curl" }
              ].map((item) => (
                <div
                  className={cx(
                    "rounded-[28px] border p-4",
                    isDark ? "border-white/10 bg-[#0b0d12]" : "border-zinc-200 bg-zinc-950"
                  )}
                  key={item.field}
                >
                  <div className="mb-3 flex items-center justify-between gap-3">
                    <p className="text-xs uppercase tracking-[0.18em] text-zinc-500">
                      {item.label}
                    </p>
                    <button
                      className={cx(
                        "inline-flex h-9 items-center gap-2 rounded-2xl px-3 text-xs transition-all duration-200",
                        isDark
                          ? "bg-white/6 text-zinc-300 hover:bg-white/10"
                          : "bg-white/10 text-zinc-100 hover:bg-white/15"
                      )}
                      onClick={() => copyText(item.value, item.field)}
                      type="button"
                    >
                      <Copy size={14} strokeWidth={1.5} />
                      {copiedField === item.field ? t.copied : t.copy}
                    </button>
                  </div>
                  <pre className="overflow-auto text-xs leading-6 text-zinc-200">
                    <code>{item.value}</code>
                  </pre>
                </div>
              ))}

              <div className="mt-6 flex flex-wrap justify-end gap-3">
                <button
                  className={cx(
                    "rounded-2xl px-4 py-3 text-sm font-medium transition-all duration-200",
                    isDark
                      ? "bg-white/6 text-zinc-300 hover:bg-white/10"
                      : "bg-zinc-100 text-zinc-600 hover:bg-zinc-200"
                  )}
                  onClick={() => {
                    setCreated(null);
                    setCopiedField("");
                  }}
                  type="button"
                >
                  {t.createAnother}
                </button>
                <button
                  className={cx(
                    "inline-flex items-center gap-2 rounded-2xl px-4 py-3 text-sm font-medium transition-all duration-200",
                    isDark
                      ? "bg-zinc-100 text-zinc-950 hover:opacity-90"
                      : "bg-zinc-900 text-white hover:opacity-90"
                  )}
                  onClick={onClose}
                  type="button"
                >
                  <CheckCircle2 size={16} strokeWidth={1.5} />
                  {t.close}
                </button>
              </div>
            </div>
          ) : (
            <>
              <div className="mt-6 space-y-4">
                <label className="block">
                  <span className={cx("mb-2 flex items-center gap-2 text-xs", isDark ? "text-zinc-500" : "text-zinc-500")}>
                    <Users size={14} strokeWidth={1.5} />
                    {t.name}
                  </span>
                  <input
                    className={cx(
                      "w-full rounded-2xl border px-4 py-3 text-sm outline-none transition-all duration-200",
                      isDark
                        ? "border-white/10 bg-white/[0.04] text-zinc-100 focus:border-sky-400"
                        : "border-zinc-200 bg-zinc-50 text-zinc-700 focus:border-sky-300"
                    )}
                    onChange={(event) => setName(event.target.value)}
                    placeholder={t.namePlaceholder}
                    type="text"
                    value={name}
                  />
                </label>

                <label className="block">
                  <span className={cx("mb-2 flex items-center gap-2 text-xs", isDark ? "text-zinc-500" : "text-zinc-500")}>
                    <Mail size={14} strokeWidth={1.5} />
                    {t.email}
                  </span>
                  <input
                    className={cx(
                      "w-full rounded-2xl border px-4 py-3 text-sm outline-none transition-all duration-200",
                      isDark
                        ? "border-white/10 bg-white/[0.04] text-zinc-100 focus:border-sky-400"
                        : "border-zinc-200 bg-zinc-50 text-zinc-700 focus:border-sky-300"
                    )}
                    onChange={(event) => setEmail(event.target.value)}
                    placeholder={t.emailPlaceholder}
                    type="email"
                    value={email}
                  />
                </label>

                <div className="grid gap-4 md:grid-cols-2">
                  <div>
                    <span className={cx("mb-2 flex items-center gap-2 text-xs", isDark ? "text-zinc-500" : "text-zinc-500")}>
                      <Shield size={14} strokeWidth={1.5} />
                      {t.role}
                    </span>
                    <div className="grid grid-cols-2 gap-3">
                      {[
                        { id: "admin" as const, label: t.admin },
                        { id: "viewer" as const, label: t.viewer }
                      ].map((item) => (
                        <button
                          className={cx(
                            "rounded-2xl px-4 py-3 text-sm font-medium transition-all duration-200",
                            role === item.id
                              ? isDark
                                ? "bg-zinc-100 text-zinc-950"
                                : "bg-zinc-900 text-white"
                              : isDark
                                ? "bg-white/[0.05] text-zinc-300 hover:bg-white/[0.08]"
                                : "bg-zinc-100 text-zinc-600 hover:bg-zinc-200"
                          )}
                          key={item.id}
                          onClick={() => setRole(item.id)}
                          type="button"
                        >
                          {item.label}
                        </button>
                      ))}
                    </div>
                  </div>

                  <div>
                    <span className={cx("mb-2 flex items-center gap-2 text-xs", isDark ? "text-zinc-500" : "text-zinc-500")}>
                      <Sparkles size={14} strokeWidth={1.5} />
                      {t.model}
                    </span>
                    <select
                      className={cx(
                        "w-full rounded-2xl border px-4 py-3 text-sm outline-none transition-all duration-200",
                        isDark
                          ? "border-white/10 bg-white/[0.04] text-zinc-100 focus:border-sky-400"
                          : "border-zinc-200 bg-zinc-50 text-zinc-700 focus:border-sky-300"
                      )}
                      onChange={(event) => setDefaultModel(event.target.value)}
                      value={defaultModel}
                    >
                      <option value="">{t.modelAuto}</option>
                      {availableModels.map((model) => (
                        <option key={model} value={model}>
                          {model}
                        </option>
                      ))}
                    </select>
                  </div>
                </div>

                <div className="grid gap-4 md:grid-cols-2">
                  <div>
                    <span className={cx("mb-2 flex items-center gap-2 text-xs", isDark ? "text-zinc-500" : "text-zinc-500")}>
                      <MoonStar size={14} strokeWidth={1.5} />
                      {t.reasoning}
                    </span>
                    <select
                      className={cx(
                        "w-full rounded-2xl border px-4 py-3 text-sm outline-none transition-all duration-200",
                        isDark
                          ? "border-white/10 bg-white/[0.04] text-zinc-100 focus:border-sky-400"
                          : "border-zinc-200 bg-zinc-50 text-zinc-700 focus:border-sky-300"
                      )}
                      onChange={(event) => setReasoningEffort(event.target.value)}
                      value={reasoningEffort}
                    >
                      <option value="">{t.reasoningAuto}</option>
                      {["low", "medium", "high", "xhigh"].map((level) => (
                        <option key={level} value={level}>
                          {level}
                        </option>
                      ))}
                    </select>
                  </div>

                  <div className="space-y-3 rounded-[24px] border border-dashed px-4 py-3">
                    {[
                      {
                        checked: forceModelOverride,
                        label: t.overrideModel,
                        onClick: () => setForceModelOverride((value) => !value)
                      },
                      {
                        checked: forceReasoningOverride,
                        label: t.overrideReasoning,
                        onClick: () => setForceReasoningOverride((value) => !value)
                      }
                    ].map((item) => (
                      <button
                        className="flex w-full items-center justify-between text-left"
                        key={item.label}
                        onClick={item.onClick}
                        type="button"
                      >
                        <span className={cx("text-sm", isDark ? "text-zinc-200" : "text-zinc-700")}>
                          {item.label}
                        </span>
                        <span
                          className={cx(
                            "relative h-7 w-12 rounded-full transition-all duration-200",
                            item.checked
                              ? "bg-sky-500"
                              : isDark
                                ? "bg-white/10"
                                : "bg-zinc-300"
                          )}
                        >
                          <span
                            className={cx(
                              "absolute top-1 h-5 w-5 rounded-full bg-white transition-all duration-200",
                              item.checked ? "left-6" : "left-1"
                            )}
                          />
                        </span>
                      </button>
                    ))}
                  </div>
                </div>

                {error ? (
                  <div
                    className={cx(
                      "rounded-[20px] px-4 py-3 text-sm",
                      isDark ? "bg-rose-500/10 text-rose-200" : "bg-rose-50 text-rose-700"
                    )}
                  >
                    {error}
                  </div>
                ) : null}
              </div>

              <div className="mt-6 flex flex-wrap justify-end gap-3">
                <button
                  className={cx(
                    "rounded-2xl px-4 py-3 text-sm font-medium transition-all duration-200",
                    isDark
                      ? "bg-white/6 text-zinc-300 hover:bg-white/10"
                      : "bg-zinc-100 text-zinc-600 hover:bg-zinc-200"
                  )}
                  onClick={onClose}
                  type="button"
                >
                  {t.cancel}
                </button>
                <button
                  className={cx(
                    "inline-flex items-center gap-2 rounded-2xl px-4 py-3 text-sm font-medium transition-all duration-200",
                    isDark
                      ? "bg-zinc-100 text-zinc-950 hover:opacity-90"
                      : "bg-zinc-900 text-white hover:opacity-90",
                    busy && "opacity-70"
                  )}
                  disabled={busy}
                  onClick={handleSubmit}
                  type="button"
                >
                  {busy ? (
                    <Sparkles size={16} strokeWidth={1.5} />
                  ) : (
                    <UserPlus size={16} strokeWidth={1.5} />
                  )}
                  {busy ? t.creating : t.create}
                </button>
              </div>
            </>
          )}
        </section>
      </div>
    </div>
  );
}
