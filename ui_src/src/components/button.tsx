import { useState } from "react";
import classNames from "classnames";

interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  onClick?: (e: React.MouseEvent) => Promise<void> | void;
  variant?: "primary" | "secondary" | "destructive";
  size?: "sm" | "default" | "lg";
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

  const baseClasses = "inline-flex items-center justify-center rounded-md font-medium transition-colors focus:outline-none focus:ring-2 focus:ring-offset-2 disabled:opacity-50 disabled:cursor-not-allowed";
  
  const variantClasses = {
    primary: "bg-neutral-700 text-white hover:bg-neutral-600 focus:ring-neutral-500",
    secondary: "bg-neutral-800 text-neutral-300 hover:bg-neutral-700 focus:ring-neutral-500",
    destructive: "bg-red-600 text-white hover:bg-red-500 focus:ring-red-500"
  };

  const sizeClasses = {
    sm: "h-8 px-3 text-sm",
    default: "h-10 px-4 py-2",
    lg: "h-12 px-6 text-lg"
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
