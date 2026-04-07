import type { ComponentProps } from "react";
import { SurfaceButton } from "@/components/buttons/SurfaceButton";

export function CreateTenantButton(props: Omit<ComponentProps<"button">, "children">) {
  return (
    <SurfaceButton className="button-create-tenant" {...props}>
      创建租户
    </SurfaceButton>
  );
}
