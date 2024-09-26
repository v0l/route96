/**
 * @constant {number} - Size of 1 kiB
 */
export const kiB = Math.pow(1024, 1);
/**
 * @constant {number} - Size of 1 MiB
 */
export const MiB = Math.pow(1024, 2);
/**
 * @constant {number} - Size of 1 GiB
 */
export const GiB = Math.pow(1024, 3);
/**
 * @constant {number} - Size of 1 TiB
 */
export const TiB = Math.pow(1024, 4);
/**
 * @constant {number} - Size of 1 PiB
 */
export const PiB = Math.pow(1024, 5);
/**
 * @constant {number} - Size of 1 EiB
 */
export const EiB = Math.pow(1024, 6);
/**
 * @constant {number} - Size of 1 ZiB
 */
export const ZiB = Math.pow(1024, 7);
/**
 * @constant {number} - Size of 1 YiB
 */
export const YiB = Math.pow(1024, 8);

export function FormatBytes(b: number, f?: number) {
  f ??= 2;
  if (b >= YiB) return (b / YiB).toFixed(f) + " YiB";
  if (b >= ZiB) return (b / ZiB).toFixed(f) + " ZiB";
  if (b >= EiB) return (b / EiB).toFixed(f) + " EiB";
  if (b >= PiB) return (b / PiB).toFixed(f) + " PiB";
  if (b >= TiB) return (b / TiB).toFixed(f) + " TiB";
  if (b >= GiB) return (b / GiB).toFixed(f) + " GiB";
  if (b >= MiB) return (b / MiB).toFixed(f) + " MiB";
  if (b >= kiB) return (b / kiB).toFixed(f) + " KiB";
  return b.toFixed(f) + " B";
}
