import type { ComponentProps, ReactNode } from "react";

type SurfaceButtonProps = ComponentProps<"button"> & {
  icon?: ReactNode;
  variant?: "primary" | "ghost";
};

export function SurfaceButton({
  children,
  className,
  icon,
  type = "submit",
  variant = "primary",
  ...props
}: SurfaceButtonProps) {
  const classes = ["button", variant, className].filter(Boolean).join(" ");

  return (
    <button className={classes} type={type} {...props}>
      {icon}
      <span>{children}</span>
    </button>
  );
}
