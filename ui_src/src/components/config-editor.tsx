/**
 * ConfigEditor — structured + raw fallback editor for the database config layer.
 *
 * Known fields are rendered with appropriate controls (number with unit picker,
 * toggle, text, etc.).  Any key not in the known-fields list falls through to
 * the raw key/value table at the bottom, which also doubles as the escape hatch
 * for arbitrary dot-notation keys.
 */

import { useState } from "react";
import { ConfigEntry } from "../upload/admin";
import { FormatBytes } from "../const";

// ── types ────────────────────────────────────────────────────────────────────

type ByteUnit = "B" | "KB" | "MB" | "GB";

interface KnownField {
  key: string;
  label: string;
  description: string;
  type: "bytes" | "text" | "url" | "bool" | "select" | "whitelist";
  options?: { value: string; label: string }[]; // for "select"
  optional?: boolean; // show a "revert to default" / clear button
}

// ── known-field registry ─────────────────────────────────────────────────────

const KNOWN_FIELDS: KnownField[] = [
  {
    key: "max_upload_bytes",
    label: "Max upload size",
    description: "Maximum file size accepted per upload.",
    type: "bytes",
  },
  {
    key: "public_url",
    label: "Public URL",
    description: "Base URL used in generated file links.",
    type: "url",
  },
  {
    key: "webhook_url",
    label: "Webhook URL",
    description: "HTTP endpoint notified on upload events.",
    type: "url",
    optional: true,
  },
  {
    key: "reject_sensitive_exif",
    label: "Reject sensitive EXIF",
    description:
      "Refuse image uploads that contain GPS coordinates or device identifiers in EXIF metadata.",
    type: "bool",
    optional: true,
  },
  {
    key: "whitelist",
    label: "Whitelist mode",
    description:
      "Controls who can upload. Open allows anyone; Database uses the Whitelist tab; File reads pubkeys from a text file (one hex key per line, # comments ignored).",
    type: "whitelist",
    optional: true,
  },
];

// Keys hidden from the UI entirely (not shown as structured fields and not
// shown in the raw fallback either — too dangerous or deployment-specific).
const HIDDEN_KEYS = new Set(["listen", "storage_dir", "database", "models_dir"]);

const KNOWN_KEYS = new Set([...KNOWN_FIELDS.map((f) => f.key), ...HIDDEN_KEYS]);

// ── byte helpers ──────────────────────────────────────────────────────────────

const UNIT_MULTIPLIERS: Record<ByteUnit, number> = {
  B: 1,
  KB: 1024,
  MB: 1024 * 1024,
  GB: 1024 * 1024 * 1024,
};

function bestUnit(bytes: number): ByteUnit {
  if (bytes >= 1024 * 1024 * 1024) return "GB";
  if (bytes >= 1024 * 1024) return "MB";
  if (bytes >= 1024) return "KB";
  return "B";
}

function toBytes(value: number, unit: ByteUnit): number {
  return Math.round(value * UNIT_MULTIPLIERS[unit]);
}

function fromBytes(bytes: number, unit: ByteUnit): number {
  return bytes / UNIT_MULTIPLIERS[unit];
}

// ── sub-components ────────────────────────────────────────────────────────────

function inputCls(extra = "") {
  return `h-7 rounded-sm border border-neutral-800 bg-neutral-950 px-2 text-xs text-neutral-300 placeholder-neutral-600 focus:outline-none focus:border-neutral-600 ${extra}`;
}

function BtnSmall({
  onClick,
  disabled,
  danger,
  children,
}: {
  onClick: () => void;
  disabled?: boolean;
  danger?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className={`px-2 py-0.5 rounded-sm text-xs transition-colors disabled:opacity-40 ${
        danger
          ? "bg-neutral-800 hover:bg-red-900 text-neutral-400 hover:text-red-200"
          : "bg-neutral-800 hover:bg-neutral-700 text-neutral-400 hover:text-white"
      }`}
    >
      {children}
    </button>
  );
}

// ── BytesField ────────────────────────────────────────────────────────────────

function BytesField({
  currentValue,
  onSave,
  onDelete,
  optional,
}: {
  currentValue: string | undefined;
  onSave: (raw: string) => Promise<void>;
  onDelete: () => Promise<void>;
  optional?: boolean;
}) {
  const parsed = currentValue !== undefined ? Number(currentValue) : NaN;
  const initUnit = !isNaN(parsed) ? bestUnit(parsed) : "MB";
  const initNum = !isNaN(parsed)
    ? fromBytes(parsed, initUnit)
    : "";

  const [num, setNum] = useState<string>(String(initNum));
  const [unit, setUnit] = useState<ByteUnit>(initUnit);
  const [saving, setSaving] = useState(false);

  const bytes = toBytes(Number(num), unit);
  const dirty =
    isNaN(parsed) || bytes !== parsed || String(initNum) !== num;

  async function save() {
    if (!num || isNaN(Number(num))) return;
    setSaving(true);
    await onSave(String(bytes));
    setSaving(false);
  }

  return (
    <div className="flex items-center gap-2">
      <input
        type="number"
        min={1}
        className={inputCls("w-28")}
        value={num}
        onChange={(e) => setNum(e.target.value)}
        onKeyDown={(e) => { if (e.key === "Enter") save(); }}
      />
      <select
        value={unit}
        onChange={(e) => setUnit(e.target.value as ByteUnit)}
        className={inputCls("w-16 cursor-pointer")}
      >
        {(["B", "KB", "MB", "GB"] as ByteUnit[]).map((u) => (
          <option key={u} value={u}>
            {u}
          </option>
        ))}
      </select>
      {currentValue !== undefined && (
        <span className="text-xs text-neutral-600">
          = {FormatBytes(bytes, 1)}
        </span>
      )}
      {dirty && (
        <BtnSmall onClick={save} disabled={saving || !num}>
          {saving ? "Saving…" : "Save"}
        </BtnSmall>
      )}
      {optional && currentValue !== undefined && (
        <BtnSmall onClick={onDelete} danger>
          Revert
        </BtnSmall>
      )}
    </div>
  );
}

// ── TextField ────────────────────────────────────────────────────────────────

function TextField({
  currentValue,
  placeholder,
  type = "text",
  onSave,
  onDelete,
  optional,
}: {
  currentValue: string | undefined;
  placeholder?: string;
  type?: "text" | "url";
  onSave: (v: string) => Promise<void>;
  onDelete: () => Promise<void>;
  optional?: boolean;
}) {
  const [val, setVal] = useState(currentValue ?? "");
  const [saving, setSaving] = useState(false);
  const dirty = val !== (currentValue ?? "");

  async function save() {
    if (!val.trim()) return;
    setSaving(true);
    await onSave(val.trim());
    setSaving(false);
  }

  return (
    <div className="flex items-center gap-2">
      <input
        type={type === "url" ? "url" : "text"}
        className={inputCls("flex-1 min-w-0")}
        value={val}
        placeholder={placeholder}
        onChange={(e) => setVal(e.target.value)}
        onKeyDown={(e) => { if (e.key === "Enter") save(); }}
      />
      {dirty && (
        <BtnSmall onClick={save} disabled={saving || !val.trim()}>
          {saving ? "Saving…" : "Save"}
        </BtnSmall>
      )}
      {optional && currentValue !== undefined && (
        <BtnSmall onClick={onDelete} danger>
          Revert
        </BtnSmall>
      )}
    </div>
  );
}

// ── BoolField ────────────────────────────────────────────────────────────────

function BoolField({
  currentValue,
  onSave,
  onDelete,
  optional,
}: {
  currentValue: string | undefined;
  onSave: (v: string) => Promise<void>;
  onDelete: () => Promise<void>;
  optional?: boolean;
}) {
  const active = currentValue === "true";
  const [saving, setSaving] = useState(false);

  async function toggle() {
    setSaving(true);
    await onSave(active ? "false" : "true");
    setSaving(false);
  }

  return (
    <div className="flex items-center gap-3">
      <button
        onClick={toggle}
        disabled={saving}
        className={`relative inline-flex h-5 w-9 items-center rounded-full transition-colors disabled:opacity-40 focus:outline-none ${
          active ? "bg-blue-600" : "bg-neutral-700"
        }`}
      >
        <span
          className={`inline-block h-3.5 w-3.5 transform rounded-full bg-white transition-transform ${
            active ? "translate-x-4.5" : "translate-x-0.5"
          }`}
        />
      </button>
      <span className="text-xs text-neutral-400">
        {currentValue === undefined
          ? "not set (using config.yaml default)"
          : active
            ? "enabled"
            : "disabled"}
      </span>
      {optional && currentValue !== undefined && (
        <BtnSmall onClick={onDelete} danger>
          Revert
        </BtnSmall>
      )}
    </div>
  );
}

// ── WhitelistField ────────────────────────────────────────────────────────────

type WhitelistMode = "open" | "database" | "file";

function parseWhitelistMode(value: string | undefined): {
  mode: WhitelistMode;
  path: string;
} {
  if (value === undefined) return { mode: "open", path: "" };
  if (value === "true") return { mode: "database", path: "" };
  return { mode: "file", path: value };
}

function WhitelistField({
  currentValue,
  onSave,
  onDelete,
}: {
  currentValue: string | undefined;
  onSave: (v: string) => Promise<void>;
  onDelete: () => Promise<void>;
}) {
  const initial = parseWhitelistMode(currentValue);
  const [mode, setMode] = useState<WhitelistMode>(initial.mode);
  const [path, setPath] = useState(initial.path);
  const [saving, setSaving] = useState(false);

  // Compute what the raw stored value would be for the current UI state
  function rawValue(): string | null {
    if (mode === "open") return null; // means delete the key
    if (mode === "database") return "true";
    return path.trim() || null;
  }

  // Is the current UI state different from what's stored?
  const raw = rawValue();
  const dirty = raw !== (currentValue ?? null);
  const canSave = dirty && (mode !== "file" || path.trim().length > 0);

  async function save() {
    setSaving(true);
    if (mode === "open") {
      await onDelete();
    } else if (raw !== null) {
      await onSave(raw);
    }
    setSaving(false);
  }

  const modeLabel: Record<WhitelistMode, string> = {
    open: "Open (no restriction)",
    database: "Database (managed via Whitelist tab)",
    file: "File (path to pubkey list)",
  };

  return (
    <div className="space-y-2">
      {/* Mode selector */}
      <div className="flex flex-col gap-1.5">
        {(["open", "database", "file"] as WhitelistMode[]).map((m) => (
          <label key={m} className="flex items-center gap-2 cursor-pointer">
            <input
              type="radio"
              name="whitelist-mode"
              value={m}
              checked={mode === m}
              onChange={() => setMode(m)}
              className="accent-blue-500"
            />
            <span className="text-xs text-neutral-300">{modeLabel[m]}</span>
          </label>
        ))}
      </div>

      {/* Path input — only shown in file mode */}
      {mode === "file" && (
        <input
          type="text"
          placeholder="/etc/route96/whitelist.txt"
          className={inputCls("w-full font-mono")}
          value={path}
          onChange={(e) => setPath(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Enter") save(); }}
        />
      )}

      {/* Action buttons */}
      <div className="flex items-center gap-2">
        {canSave && (
          <BtnSmall onClick={save} disabled={saving}>
            {saving ? "Saving…" : "Save"}
          </BtnSmall>
        )}
        {currentValue !== undefined && (
          <BtnSmall
            onClick={async () => {
              setSaving(true);
              await onDelete();
              setMode("open");
              setPath("");
              setSaving(false);
            }}
            danger
          >
            Revert to open
          </BtnSmall>
        )}
      </div>
    </div>
  );
}

// ── RawEditor — bare key/value fallback ──────────────────────────────────────

function RawEditor({
  unknown,
  onSave,
  onDelete,
}: {
  unknown: ConfigEntry[];
  onSave: (key: string, value: string) => Promise<void>;
  onDelete: (key: string) => Promise<void>;
}) {
  const [key, setKey] = useState("");
  const [value, setValue] = useState("");
  const [editingKey, setEditingKey] = useState<string>();
  const [saving, setSaving] = useState(false);

  function startEdit(e: ConfigEntry) {
    setEditingKey(e.key);
    setKey(e.key);
    setValue(e.value);
  }

  function cancelEdit() {
    setEditingKey(undefined);
    setKey("");
    setValue("");
  }

  async function save() {
    if (!key.trim() || !value.trim()) return;
    setSaving(true);
    await onSave(key.trim(), value.trim());
    setSaving(false);
    setKey("");
    setValue("");
    setEditingKey(undefined);
  }

  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2 flex-wrap">
        <input
          type="text"
          placeholder="Key (dot-notation)"
          className={inputCls("w-52 font-mono")}
          value={key}
          disabled={editingKey !== undefined}
          onChange={(e) => setKey(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Enter") save(); }}
        />
        <input
          type="text"
          placeholder="Value"
          className={inputCls("flex-1 min-w-32 font-mono")}
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Enter") save(); }}
        />
        <button
          onClick={save}
          disabled={saving || !key.trim() || !value.trim()}
          className="bg-neutral-800 hover:bg-neutral-700 disabled:opacity-40 text-white px-3 h-7 rounded-sm text-xs"
        >
          {editingKey ? "Save" : "Add"}
        </button>
        {editingKey && (
          <button
            onClick={cancelEdit}
            className="bg-neutral-800 hover:bg-neutral-700 text-neutral-400 px-3 h-7 rounded-sm text-xs"
          >
            Cancel
          </button>
        )}
      </div>

      {unknown.length > 0 && (
        <div className="space-y-1">
          {unknown.map((entry) => (
            <div
              key={entry.key}
              className="flex items-center justify-between bg-neutral-900 border border-neutral-800 rounded-sm px-3 py-2"
            >
              <div className="min-w-0 flex-1">
                <span className="font-mono text-xs text-neutral-300">
                  {entry.key}
                </span>
                <span className="mx-2 text-neutral-700 text-xs">=</span>
                <span className="font-mono text-xs text-neutral-500">
                  {entry.value}
                </span>
              </div>
              <div className="ml-3 shrink-0 flex gap-1">
                <BtnSmall onClick={() => startEdit(entry)}>Edit</BtnSmall>
                <BtnSmall onClick={() => onDelete(entry.key)} danger>
                  Delete
                </BtnSmall>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

// ── main export ───────────────────────────────────────────────────────────────

export default function ConfigEditor({
  config,
  onSave,
  onDelete,
}: {
  config: ConfigEntry[];
  onSave: (key: string, value: string) => Promise<void>;
  onDelete: (key: string) => Promise<void>;
}) {
  // Index current overrides by key for O(1) lookup
  const overrideMap = new Map(config.map((e) => [e.key, e.value]));

  // Entries that don't match any known field go to the raw fallback section
  const unknown = config.filter((e) => !KNOWN_KEYS.has(e.key));

  function renderField(field: KnownField) {
    const current = overrideMap.get(field.key);

    const save = (raw: string) => onSave(field.key, raw);
    const del = () => onDelete(field.key);

    let control: React.ReactNode;
    if (field.type === "bytes") {
      control = (
        <BytesField
          currentValue={current}
          onSave={save}
          onDelete={del}
          optional={field.optional}
        />
      );
    } else if (field.type === "bool") {
      control = (
        <BoolField
          currentValue={current}
          onSave={save}
          onDelete={del}
          optional={field.optional}
        />
      );
    } else if (field.type === "whitelist") {
      control = (
        <WhitelistField
          currentValue={current}
          onSave={save}
          onDelete={del}
        />
      );
    } else {
      control = (
        <TextField
          currentValue={current}
          type={field.type === "url" ? "url" : "text"}
          onSave={save}
          onDelete={del}
          optional={field.optional}
        />
      );
    }

    const isOverridden = current !== undefined;

    return (
      <div
        key={field.key}
        className={`rounded-sm border px-4 py-3 space-y-2 ${
          isOverridden
            ? "border-blue-900 bg-blue-950/20"
            : "border-neutral-800 bg-neutral-900/40"
        }`}
      >
        <div className="flex items-center justify-between gap-2">
          <div>
            <span className="text-xs font-medium text-neutral-200">
              {field.label}
            </span>
            {isOverridden && (
              <span className="ml-2 text-[10px] text-blue-400 bg-blue-900/40 px-1.5 py-0.5 rounded-sm">
                overridden
              </span>
            )}
          </div>
          <code className="text-[10px] text-neutral-600 font-mono">
            {field.key}
          </code>
        </div>
        <p className="text-xs text-neutral-500">{field.description}</p>
        {control}
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {/* Structured fields */}
      <div className="space-y-2">{KNOWN_FIELDS.map(renderField)}</div>

      {/* Raw fallback */}
      <div className="space-y-3">
        <div className="flex items-center gap-3">
          <div className="flex-1 h-px bg-neutral-800" />
          <span className="text-xs text-neutral-600">Raw key/value</span>
          <div className="flex-1 h-px bg-neutral-800" />
        </div>
        <p className="text-xs text-neutral-600">
          Set arbitrary dot-notation keys not covered above (e.g.{" "}
          <code className="font-mono text-neutral-500">payments.cost</code>
          ). Values are parsed as boolean, integer, float, or string.
        </p>
        <RawEditor
          unknown={unknown}
          onSave={onSave}
          onDelete={onDelete}
        />
      </div>
    </div>
  );
}
