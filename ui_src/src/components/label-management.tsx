import { useEffect, useState, useCallback } from "react";
import { Route96 } from "../upload/admin";
import { EventPublisher } from "@snort/system";

interface LabelModel {
  name: string;
  type: string;
  config: string;
}

const inputCls = (extra = "") =>
  `bg-neutral-950 border border-neutral-800 rounded-sm px-3 py-2 text-sm text-neutral-200 placeholder-neutral-600 focus:outline-none focus:ring-2 focus:ring-blue-500/50 ${extra}`;

const BtnSmall = ({
  onClick,
  disabled,
  danger,
  children,
}: {
  onClick?: () => void;
  disabled?: boolean;
  danger?: boolean;
  children: React.ReactNode;
}) => (
  <button
    onClick={onClick}
    disabled={disabled}
    className={`px-2 py-1 rounded-sm text-xs ${
      danger
        ? "bg-red-900/30 hover:bg-red-900/50 text-red-400 border border-red-800"
        : "bg-neutral-800 hover:bg-neutral-700 text-neutral-200 border border-neutral-700"
    } disabled:opacity-50 disabled:cursor-not-allowed`}
  >
    {children}
  </button>
);

// ── Label Flag Terms Section ────────────────────────────────────────────────

function LabelFlagTerms({
  pub,
  url,
  onDirty,
}: {
  pub: EventPublisher;
  url: string;
  onDirty?: (isDirty: boolean) => void;
}) {
  const [terms, setTerms] = useState<string[]>([]);
  const [inputTerm, setInputTerm] = useState("");
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string>();
  const [dirty, setDirty] = useState(false);

  const loadTerms = useCallback(async () => {
    try {
      const route96 = new Route96(url, pub);
      const result = await route96.getLabelFlagTerms();
      setTerms(result || []);
      setError(undefined);
    } catch (e) {
      setError(
        e instanceof Error ? e.message : "Failed to load label flag terms"
      );
    } finally {
      setLoading(false);
    }
  }, [pub, url]);

  useEffect(() => {
    loadTerms();
  }, [loadTerms]);

  useEffect(() => {
    onDirty?.(dirty);
  }, [dirty, onDirty]);

  const handleAddTerm = () => {
    const trimmed = inputTerm.trim();
    if (trimmed && !terms.includes(trimmed)) {
      setTerms([...terms, trimmed]);
      setInputTerm("");
      setDirty(true);
    }
  };

  const handleRemoveTerm = (term: string) => {
    setTerms(terms.filter((t) => t !== term));
    setDirty(true);
  };

  const handleSave = async () => {
    if (terms.length === 0) {
      setError("Please add at least one term");
      return;
    }
    try {
      setSaving(true);
      setError(undefined);
      const route96 = new Route96(url, pub);
      await route96.setLabelFlagTerms(terms);
      setDirty(false);
    } catch (e) {
      setError(
        e instanceof Error ? e.message : "Failed to save label flag terms"
      );
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async () => {
    try {
      setSaving(true);
      setError(undefined);
      const route96 = new Route96(url, pub);
      await route96.deleteLabelFlagTerms();
      setTerms([]);
      setDirty(false);
    } catch (e) {
      setError(
        e instanceof Error ? e.message : "Failed to delete label flag terms"
      );
    } finally {
      setSaving(false);
    }
  };

  if (loading) {
    return (
      <div className="text-sm text-neutral-500 py-4">Loading...</div>
    );
  }

  return (
    <div className="space-y-4">
      <div>
        <h3 className="text-sm font-medium text-neutral-300 mb-2">
          Label Flag Terms
        </h3>
        <p className="text-xs text-neutral-500 mb-3">
          Terms that, when found in AI-generated labels, will automatically flag
          the file for review. For example: nsfw, sensitive, adult
        </p>

        <div className="flex gap-2 mb-3">
          <input
            type="text"
            value={inputTerm}
            onChange={(e) => setInputTerm(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && handleAddTerm()}
            placeholder="Add a term"
            className={inputCls("flex-1")}
          />
          <BtnSmall onClick={handleAddTerm} disabled={!inputTerm.trim()}>
            Add
          </BtnSmall>
        </div>

        {terms.length > 0 && (
          <div className="flex flex-wrap gap-2 mb-3">
            {terms.map((term) => (
              <span
                key={term}
                className="inline-flex items-center gap-1 bg-neutral-800 border border-neutral-700 rounded-sm px-2 py-1 text-xs text-neutral-300"
              >
                {term}
                <button
                  onClick={() => handleRemoveTerm(term)}
                  className="text-neutral-500 hover:text-red-400"
                >
                  ×
                </button>
              </span>
            ))}
          </div>
        )}

        {error && (
          <div className="text-xs text-red-400 mb-3">{error}</div>
        )}

        <div className="flex gap-2">
          {dirty && (
            <BtnSmall onClick={handleSave} disabled={saving || terms.length === 0}>
              {saving ? "Saving…" : "Save"}
            </BtnSmall>
          )}
          {terms.length > 0 && (
            <BtnSmall onClick={handleDelete} danger disabled={saving}>
              Revert to Default
            </BtnSmall>
          )}
        </div>
      </div>
    </div>
  );
}

// ── Label Models Section ────────────────────────────────────────────────────

function LabelModels({
  pub,
  url,
}: {
  pub: EventPublisher;
  url: string;
}) {
  const [models, setModels] = useState<LabelModel[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string>();
  const [newModel, setNewModel] = useState<LabelModel>({
    name: "",
    type: "",
    config: "",
  });
  const [saving, setSaving] = useState(false);

  const loadModels = useCallback(async () => {
    try {
      const route96 = new Route96(url, pub);
      const result = await route96.listLabelModels();
      setModels(result);
      setError(undefined);
    } catch (e) {
      setError(
        e instanceof Error ? e.message : "Failed to load label models"
      );
    } finally {
      setLoading(false);
    }
  }, [pub, url]);

  useEffect(() => {
    loadModels();
  }, [loadModels]);

  const handleAddModel = async () => {
    if (!newModel.name || !newModel.type || !newModel.config) {
      setError("Please fill in all fields");
      return;
    }
    try {
      setSaving(true);
      setError(undefined);
      const route96 = new Route96(url, pub);
      await route96.addLabelModel(newModel);
      setNewModel({ name: "", type: "", config: "" });
      await loadModels();
    } catch (e) {
      setError(
        e instanceof Error ? e.message : "Failed to add label model"
      );
    } finally {
      setSaving(false);
    }
  };

  const handleRemoveModel = async (name: string) => {
    try {
      setSaving(true);
      setError(undefined);
      const route96 = new Route96(url, pub);
      await route96.removeLabelModel(name);
      await loadModels();
    } catch (e) {
      setError(
        e instanceof Error ? e.message : "Failed to remove label model"
      );
    } finally {
      setSaving(false);
    }
  };

  if (loading) {
    return (
      <div className="text-sm text-neutral-500 py-4">Loading...</div>
    );
  }

  return (
    <div className="space-y-4">
      <div>
        <h3 className="text-sm font-medium text-neutral-300 mb-2">
          Label Models
        </h3>
        <p className="text-xs text-neutral-500 mb-3">
          Configure AI models for automatic image labeling. Supported types:
          vit (vision transformer), generic_llm (generic text labeler), and
          custom (custom API endpoint).
        </p>

        <div className="grid grid-cols-1 md:grid-cols-3 gap-2 mb-3">
          <input
            type="text"
            value={newModel.name}
            onChange={(e) =>
              setNewModel({ ...newModel, name: e.target.value })
            }
            placeholder="Model name"
            className={inputCls()}
          />
          <input
            type="text"
            value={newModel.type}
            onChange={(e) =>
              setNewModel({ ...newModel, type: e.target.value })
            }
            placeholder="Type (vit, generic_llm, custom)"
            className={inputCls()}
          />
          <input
            type="text"
            value={newModel.config}
            onChange={(e) =>
              setNewModel({ ...newModel, config: e.target.value })
            }
            placeholder="Config JSON"
            className={inputCls()}
          />
        </div>

        <div className="flex gap-2 mb-4">
          <BtnSmall onClick={handleAddModel} disabled={saving}>
            {saving ? "Adding…" : "Add Model"}
          </BtnSmall>
        </div>

        {error && <div className="text-xs text-red-400 mb-3">{error}</div>}

        {models.length > 0 && (
          <div className="space-y-2">
            <div className="text-xs text-neutral-500">
              {models.length} model{models.length !== 1 ? "s" : ""} configured
            </div>
            <div className="space-y-2">
              {models.map((model) => (
                <div
                  key={model.name}
                  className="bg-neutral-950 border border-neutral-800 rounded-sm p-3"
                >
                  <div className="flex items-center justify-between mb-2">
                    <div>
                      <div className="text-sm font-medium text-neutral-300">
                        {model.name}
                      </div>
                      <div className="text-xs text-neutral-500">
                        Type: {model.type}
                      </div>
                    </div>
                    <BtnSmall
                      onClick={() => handleRemoveModel(model.name)}
                      danger
                      disabled={saving}
                    >
                      Remove
                    </BtnSmall>
                  </div>
                  <div className="text-xs text-neutral-600 font-mono bg-neutral-900 rounded-sm p-2 overflow-x-auto">
                    {model.config}
                  </div>
                </div>
              ))}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

// ── Main Label Management Component ─────────────────────────────────────────

type Tab = "flag-terms" | "models";

export default function LabelManagement({
  pub,
  url,
}: {
  pub: EventPublisher;
  url: string;
}) {
  const [tab, setTab] = useState<Tab>("flag-terms");

  return (
    <div className="space-y-4">
      <div className="flex gap-2 border-b border-neutral-800 pb-2">
        <button
          onClick={() => setTab("flag-terms")}
          className={`px-3 py-1 text-xs rounded-sm ${
            tab === "flag-terms"
              ? "bg-blue-900/30 text-blue-400 border border-blue-800"
              : "bg-neutral-800 text-neutral-400 border border-neutral-700 hover:text-neutral-200"
          }`}
        >
          Flag Terms
        </button>
        <button
          onClick={() => setTab("models")}
          className={`px-3 py-1 text-xs rounded-sm ${
            tab === "models"
              ? "bg-blue-900/30 text-blue-400 border border-blue-800"
              : "bg-neutral-800 text-neutral-400 border border-neutral-700 hover:text-neutral-200"
          }`}
        >
          Label Models
        </button>
      </div>

      {tab === "flag-terms" && (
        <LabelFlagTerms pub={pub} url={url} />
      )}
      {tab === "models" && <LabelModels pub={pub} url={url} />}
    </div>
  );
}
