import { HTMLProps, useState } from "react";

export default function Button({
  children,
  className,
  onClick,
  ...props
}: { onClick?: (e: React.MouseEvent) => Promise<void> | void } & Omit<
  HTMLProps<HTMLButtonElement>,
  "type" | "onClick"
>) {
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
  return (
    <button
      className={`${className} ${props.disabled || loading ? "opacity-50 cursor-not-allowed" : ""}`}
      onClick={doClick}
      {...props}
      disabled={loading || (props.disabled ?? false)}
    >
      {loading ? "..." : children}
    </button>
  );
}
