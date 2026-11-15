import { useCallback, useEffect, useState } from "react";
import { CleanupRule, CleanupRuleSetting, Route96 } from "../upload/admin";

interface DeletionRulesProps {
  pub: any;
  url: string;
  onOpenLabeling: () => void;
}

const inputCls =
  "bg-neutral-950 border border-neutral-700 rounded-sm px-2 py-1 text-sm text-white w-24";

function settingLabel(key: string) {
  return key.replace(/_/g, " ");
}

function SettingRow({
  setting,
  onSave,
  onOpenLabeling,
}: {
  setting: CleanupRuleSetting;
  onSave: (key: string, value: string) => Promise<void>;
  onOpenLabeling: () => void;
}) {
  const initial =
    typeof setting.value === "number" ? String(setting.value) : "";
  const [val, setVal] = useState(initial);
  const [saving, setSaving] = useState(false);
  useEffect(() => setVal(initial), [initial]);

  const label = (
    <span className="text-xs text-neutral-400 w-72 shrink-0">
      {settingLabel(setting.key)}
    </span>
  );

  if (typeof setting.value === "number" || setting.value === null) {
    const num = Number(val);
    const fractional = !Number.isInteger(Number(initial || 0));
    const valid =
      val !== "" && num >= 0 && (fractional || Number.isInteger(num));
    const dirty = val !== initial;
    return (
      <div className="flex items-center gap-2">
        {label}
        <input
          type="number"
          min={0}
          step={fractional ? "any" : 1}
          className={inputCls}
          value={val}
          placeholder="off"
          onChange={(e) => setVal(e.target.value)}
        />
        {dirty && (
          <button
            disabled={!valid || saving}
            onClick={async () => {
              setSaving(true);
              await onSave(setting.key, String(num));
              setSaving(false);
            }}
            className="bg-neutral-800 hover:bg-neutral-700 text-white px-2 py-1 rounded-sm text-xs disabled:opacity-50"
          >
            {saving ? "Saving..." : "Save"}
          </button>
        )}
      </div>
    );
  }

  if (typeof setting.value === "boolean") {
    return (
      <div className="flex items-center gap-2">
        {label}
        <input
          type="checkbox"
          checked={setting.value}
          onChange={(e) => onSave(setting.key, String(e.target.checked))}
        />
      </div>
    );
  }

  return (
    <div className="flex items-center gap-2">
      {label}
      <span className="text-xs text-neutral-300">
        {Array.isArray(setting.value)
          ? setting.value.length > 0
            ? setting.value.join(", ")
            : "none"
          : String(setting.value)}
      </span>
      {Array.isArray(setting.value) && (
        <button
          onClick={onOpenLabeling}
          className="text-xs text-neutral-500 hover:text-neutral-300 underline"
        >
          edit in Labeling
        </button>
      )}
    </div>
  );
}

export default function DeletionRules({
  pub,
  url,
  onOpenLabeling,
}: DeletionRulesProps) {
  const [rules, setRules] = useState<CleanupRule[]>();
  const [error, setError] = useState<string>();

  const load = useCallback(async () => {
    try {
      setRules(await new Route96(url, pub).listCleanupRules());
      setError(undefined);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to load rules");
    }
  }, [pub, url]);

  useEffect(() => {
    load();
  }, [load]);

  async function save(key: string, value: string) {
    try {
      await new Route96(url, pub).setConfig(key, value);
      await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Save failed");
    }
  }

  return (
    <div className="space-y-3">
      {error && (
        <div className="bg-red-950 border border-red-900 text-red-200 px-3 py-2 rounded-sm text-sm">
          {error}
        </div>
      )}
      {!rules && !error && (
        <div className="text-sm text-neutral-500">Loading...</div>
      )}
      {rules?.map((rule) => (
        <div
          key={rule.name}
          className="bg-neutral-900 border border-neutral-800 rounded-sm p-3 space-y-2"
        >
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <span className="text-sm text-white font-medium">
                {settingLabel(rule.name)}
              </span>
              <span
                className={
                  rule.enabled
                    ? "text-xs text-green-400"
                    : "text-xs text-neutral-500"
                }
              >
                {rule.enabled ? "Enabled" : "Off"}
              </span>
            </div>
            {rule.files_deleted !== null && (
              <span className="text-xs text-neutral-500">
                {rule.files_deleted.toLocaleString()} files deleted
              </span>
            )}
          </div>
          <div className="text-xs text-neutral-400">{rule.description}</div>
          <div className="space-y-1">
            {rule.settings.map((s) => (
              <SettingRow
                key={s.key}
                setting={s}
                onSave={save}
                onOpenLabeling={onOpenLabeling}
              />
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}
