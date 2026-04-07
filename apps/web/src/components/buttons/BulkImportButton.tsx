import type { ComponentProps } from "react";
import { SurfaceButton } from "@/components/buttons/SurfaceButton";

export function BulkImportButton(props: Omit<ComponentProps<"button">, "children">) {
  return (
    <SurfaceButton className="button-bulk-import" {...props}>
      批量归档
    </SurfaceButton>
  );
}
