import type { ComponentProps } from "react";
import { SurfaceButton } from "@/components/buttons/SurfaceButton";

export function ParseCallbackButton(props: Omit<ComponentProps<"button">, "children">) {
  return (
    <SurfaceButton className="button-parse-callback" {...props}>
      解析回调地址
    </SurfaceButton>
  );
}
