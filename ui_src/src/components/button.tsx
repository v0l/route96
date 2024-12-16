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
      onClick(e);
    } finally {
      setLoading(false);
    }
  }
  return (
    <button
      className={`py-2 px-4 rounded-md border-0 text-sm font-semibold bg-neutral-700 hover:bg-neutral-600 ${className} ${props.disabled ? "opacity-50" : ""}`}
      onClick={doClick}
      {...props}
      disabled={loading || (props.disabled ?? false)}
    >
      {children}
    </button>
  );
}
