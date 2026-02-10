import { useState } from "react";
import classNames from "classnames";

interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  onClick?: (e: React.MouseEvent) => Promise<void> | void;
  variant?: "primary" | "secondary" | "destructive";
  size?: "sm" | "default";
}

export default function Button({
  children,
  onClick,
  disabled,
  className,
  variant = "primary",
  size = "default",
  ...props
}: ButtonProps) {
  const [loading, setLoading] = useState(false);

  async function doClick(e: React.MouseEvent) {
    if (!onClick) return;
    try {
      setLoading(true);
      await onClick(e);
    } finally {
      setLoading(false);
    }
  }

  const baseClasses = "inline-flex items-center justify-center rounded-sm font-medium transition-colors focus:outline-none focus:ring-1 focus:ring-neutral-500 disabled:opacity-50 disabled:cursor-not-allowed";
  
  const variantClasses = {
    primary: "bg-white text-black hover:bg-neutral-200",
    secondary: "bg-neutral-800 text-neutral-100 border border-neutral-700 hover:bg-neutral-700",
    destructive: "bg-red-600 text-white hover:bg-red-500"
  };

  const sizeClasses = {
    sm: "h-7 px-2 text-xs",
    default: "h-8 px-3 py-1.5 text-sm"
  };

  return (
    <button
      onClick={doClick}
      disabled={loading || disabled}
      className={classNames(
        baseClasses,
        variantClasses[variant],
        sizeClasses[size],
        className
      )}
      {...props}
    >
      {loading ? "..." : children}
    </button>
  );
}
