import type { ComponentProps } from "react";
import { SurfaceButton } from "@/components/buttons/SurfaceButton";

export function BrowserRecoverButton(
  props: Omit<ComponentProps<"button">, "children">
) {
  return (
    <SurfaceButton className="button-browser-recover" variant="ghost" {...props}>
      浏览器恢复
    </SurfaceButton>
  );
}
