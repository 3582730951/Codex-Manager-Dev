import type { ComponentProps } from "react";
import { SurfaceButton } from "@/components/buttons/SurfaceButton";

export function ImportAccountButton(props: Omit<ComponentProps<"button">, "children">) {
  return (
    <SurfaceButton className="button-import-account" {...props}>
      导入到控制面
    </SurfaceButton>
  );
}
