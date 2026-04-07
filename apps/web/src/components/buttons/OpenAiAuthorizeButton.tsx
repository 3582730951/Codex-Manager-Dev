import type { ComponentProps } from "react";
import { SurfaceButton } from "@/components/buttons/SurfaceButton";

export function OpenAiAuthorizeButton(props: Omit<ComponentProps<"button">, "children">) {
  return (
    <SurfaceButton className="button-openai-authorize" {...props}>
      跳转 OpenAI 授权
    </SurfaceButton>
  );
}
