import { useEffect, useState, useCallback } from "react";
import { Route96, LabelDeletion } from "../upload/admin";
import { EventPublisher } from "@snort/system";

interface LabelModel {
  name: string;
  type: string;
  config: string;
}

interface LabelModelInput {
  name: string;
  model_type: string;
  hf_repo: string;
  api_url: string;
  llm_model: string;
  api_key: string;
  prompt: string;
  label_exclude: string;
  min_confidence: string;
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

// ── Label Delete Terms Section ──────────────────────────────────────────────

function LabelDeleteTerms({
  pub,
  url,
}: {
  pub: EventPublisher;
  url: string;
}) {
  const [terms, setTerms] = useState<string[]>([]);
  const [minConfidence, setMinConfidence] = useState(0.8);
  const [dryRun, setDryRun] = useState(true);
  const [inputTerm, setInputTerm] = useState("");
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string>();
  const [dirty, setDirty] = useState(false);

  const loadTerms = useCallback(async () => {
    try {
      const route96 = new Route96(url, pub);
      const result = await route96.getLabelDeleteTerms();
      setTerms(result?.terms ?? []);
      setMinConfidence(result?.min_confidence ?? 0.8);
      setDryRun(result?.dry_run ?? true);
      setError(undefined);
    } catch (e) {
      setError(
        e instanceof Error ? e.message : "Failed to load label delete terms",
      );
    } finally {
      setLoading(false);
    }
  }, [pub, url]);

  useEffect(() => {
    loadTerms();
  }, [loadTerms]);

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
      await route96.setLabelDeleteTerms(terms);
      setDirty(false);
    } catch (e) {
      setError(
        e instanceof Error ? e.message : "Failed to save label delete terms",
      );
    } finally {
      setSaving(false);
    }
  };

  const handleClear = async () => {
    try {
      setSaving(true);
      setError(undefined);
      const route96 = new Route96(url, pub);
      await route96.deleteLabelDeleteTerms();
      setTerms([]);
      setDirty(false);
    } catch (e) {
      setError(
        e instanceof Error ? e.message : "Failed to clear label delete terms",
      );
    } finally {
      setSaving(false);
    }
  };

  if (loading) {
    return <div className="text-sm text-neutral-500 py-4">Loading...</div>;
  }

  return (
    <div className="space-y-4">
      <div>
        <h3 className="text-sm font-medium text-neutral-300 mb-2">
          Delete terms
        </h3>
        <p className="text-xs text-neutral-500 mb-3">
          Files whose AI labels contain one of these terms at or above the
          minimum confidence are deleted and their hash is banned. Reviewed
          files are never deleted.
        </p>

        <div className="flex gap-3 mb-3 text-xs">
          <span className="bg-neutral-800 border border-neutral-700 rounded-sm px-2 py-1 text-neutral-300">
            Min confidence: {minConfidence}
          </span>
          <span
            className={`rounded-sm px-2 py-1 border ${
              dryRun
                ? "bg-yellow-900/30 text-yellow-400 border-yellow-800"
                : "bg-red-900/30 text-red-400 border-red-800"
            }`}
          >
            {dryRun ? "Dry run on (nothing is deleted)" : "Dry run off (deleting)"}
          </span>
        </div>

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

        {error && <div className="text-xs text-red-400 mb-3">{error}</div>}

        <div className="flex gap-2">
          {dirty && (
            <BtnSmall
              onClick={handleSave}
              disabled={saving || terms.length === 0}
            >
              {saving ? "Saving…" : "Save"}
            </BtnSmall>
          )}
          {terms.length > 0 && (
            <BtnSmall onClick={handleClear} danger disabled={saving}>
              Clear
            </BtnSmall>
          )}
        </div>
      </div>
    </div>
  );
}

// ── Label Deletions Section ─────────────────────────────────────────────────

const DELETIONS_PAGE_SIZE = 20;

function LabelDeletions({
  pub,
  url,
}: {
  pub: EventPublisher;
  url: string;
}) {
  const [dryRun, setDryRun] = useState(true);
  const [page, setPage] = useState(0);
  const [rows, setRows] = useState<LabelDeletion[]>([]);
  const [hasMore, setHasMore] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string>();

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    const route96 = new Route96(url, pub);
    route96
      .listLabelDeletions(
        dryRun,
        page * DELETIONS_PAGE_SIZE,
        DELETIONS_PAGE_SIZE + 1,
      )
      .then((result) => {
        if (cancelled) return;
        setHasMore(result.length > DELETIONS_PAGE_SIZE);
        setRows(result.slice(0, DELETIONS_PAGE_SIZE));
        setError(undefined);
      })
      .catch((e) => {
        if (cancelled) return;
        setError(
          e instanceof Error ? e.message : "Failed to load label deletions",
        );
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [pub, url, dryRun, page]);

  const choose = (value: boolean) => {
    setDryRun(value);
    setPage(0);
  };

  const tabCls = (active: boolean) =>
    `px-3 py-1 text-xs rounded-sm ${
      active
        ? "bg-blue-900/30 text-blue-400 border border-blue-800"
        : "bg-neutral-800 text-neutral-400 border border-neutral-700 hover:text-neutral-200"
    }`;

  return (
    <div className="space-y-4">
      <div>
        <h3 className="text-sm font-medium text-neutral-300 mb-2">
          Label deletions
        </h3>
        <div className="flex gap-2 mb-3">
          <button onClick={() => choose(true)} className={tabCls(dryRun)}>
            Would delete (dry run)
          </button>
          <button onClick={() => choose(false)} className={tabCls(!dryRun)}>
            Deleted
          </button>
        </div>

        {error && <div className="text-xs text-red-400 mb-3">{error}</div>}

        {loading ? (
          <div className="text-sm text-neutral-500 py-4">Loading...</div>
        ) : rows.length === 0 ? (
          <div className="text-xs text-neutral-500">Nothing here yet.</div>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-xs text-left">
              <thead className="text-neutral-500">
                <tr>
                  <th className="py-1 pr-3">Hash</th>
                  <th className="py-1 pr-3">Term</th>
                  <th className="py-1 pr-3">Label</th>
                  <th className="py-1 pr-3">Model</th>
                  <th className="py-1 pr-3">Confidence</th>
                  <th className="py-1 pr-3">Uploader</th>
                  <th className="py-1">Time</th>
                </tr>
              </thead>
              <tbody className="text-neutral-300">
                {rows.map((r, i) => (
                  <tr
                    key={`${r.file}-${r.created}-${i}`}
                    className="border-t border-neutral-800"
                  >
                    <td className="py-1 pr-3 font-mono">
                      <a
                        href={`${url.replace(/\/$/, "")}/${r.file}`}
                        target="_blank"
                        className="text-blue-400 hover:underline"
                        title={r.file}
                      >
                        {r.file.slice(0, 12)}…
                      </a>
                    </td>
                    <td className="py-1 pr-3">{r.term}</td>
                    <td className="py-1 pr-3">{r.label}</td>
                    <td className="py-1 pr-3">{r.model}</td>
                    <td className="py-1 pr-3">{r.confidence.toFixed(2)}</td>
                    <td className="py-1 pr-3 font-mono">
                      {r.pubkeys
                        ? r.pubkeys
                            .split(",")
                            .map((p) => `${p.slice(0, 8)}…`)
                            .join(", ")
                        : "-"}
                    </td>
                    <td className="py-1">
                      {r.created ? new Date(r.created).toLocaleString() : "-"}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}

        <div className="flex items-center gap-2 mt-3">
          <BtnSmall onClick={() => setPage(page - 1)} disabled={page === 0}>
            Previous
          </BtnSmall>
          <span className="text-xs text-neutral-500">Page {page + 1}</span>
          <BtnSmall onClick={() => setPage(page + 1)} disabled={!hasMore}>
            Next
          </BtnSmall>
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
  const [newModel, setNewModel] = useState<LabelModelInput>({
    name: "",
    model_type: "",
    hf_repo: "",
    api_url: "",
    llm_model: "",
    api_key: "",
    prompt: "",
    label_exclude: "",
    min_confidence: "",
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
    if (!newModel.name || !newModel.model_type) {
      setError("Please fill in model name and type");
      return;
    }

    // Validate type-specific required fields
    if (newModel.model_type === "vit" && !newModel.hf_repo) {
      setError("ViT model requires HuggingFace repo ID");
      return;
    }
    if (
      newModel.model_type === "generic_llm" &&
      (!newModel.api_url || !newModel.llm_model)
    ) {
      setError("LLM model requires API URL and model name");
      return;
    }

    try {
      setSaving(true);
      setError(undefined);
      const route96 = new Route96(url, pub);
      await route96.addLabelModel(newModel);
      setNewModel({
        name: "",
        model_type: "",
        hf_repo: "",
        api_url: "",
        llm_model: "",
        api_key: "",
        prompt: "",
        label_exclude: "",
        min_confidence: "",
      });
      await loadModels();
    } catch (e) {
      setError(
        e instanceof Error ? e.message : "Failed to add label model"
      );
    } finally {
      setSaving(false);
    }
  };

  const renderTypeSpecificFields = () => {
    if (newModel.model_type === "vit") {
      return (
        <>
          <div className="md:col-span-2">
            <label className="block text-xs text-neutral-400 mb-1">
              HuggingFace Repo ID
            </label>
            <input
              type="text"
              value={newModel.hf_repo}
              onChange={(e) =>
                setNewModel({ ...newModel, hf_repo: e.target.value })
              }
              placeholder="e.g., google/vit-base-patch16-224"
              className={inputCls()}
            />
          </div>
        </>
      );
    }
    if (newModel.model_type === "generic_llm") {
      return (
        <>
          <div className="md:col-span-2">
            <label className="block text-xs text-neutral-400 mb-1">
              API URL
            </label>
            <input
              type="text"
              value={newModel.api_url}
              onChange={(e) =>
                setNewModel({ ...newModel, api_url: e.target.value })
              }
              placeholder="e.g., https://api.openai.com/v1"
              className={inputCls()}
            />
          </div>
          <div>
            <label className="block text-xs text-neutral-400 mb-1">
              Model Name
            </label>
            <input
              type="text"
              value={newModel.llm_model}
              onChange={(e) =>
                setNewModel({ ...newModel, llm_model: e.target.value })
              }
              placeholder="e.g., gpt-4-vision-preview"
              className={inputCls()}
            />
          </div>
          <div>
            <label className="block text-xs text-neutral-400 mb-1">
              API Key (optional)
            </label>
            <input
              type="password"
              value={newModel.api_key}
              onChange={(e) =>
                setNewModel({ ...newModel, api_key: e.target.value })
              }
              placeholder="API key"
              className={inputCls()}
            />
          </div>
          <div className="md:col-span-3">
            <label className="block text-xs text-neutral-400 mb-1">
              Custom Prompt (optional)
            </label>
            <textarea
              value={newModel.prompt}
              onChange={(e) =>
                setNewModel({ ...newModel, prompt: e.target.value })
              }
              placeholder="Custom prompt for the LLM"
              className={inputCls("min-h-[60px] resize-none")}
            />
          </div>
        </>
      );
    }
    return null;
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
          <select
            value={newModel.model_type}
            onChange={(e) =>
              setNewModel({ ...newModel, model_type: e.target.value })
            }
            className={inputCls()}
          >
            <option value="">Select type</option>
            <option value="vit">ViT (Vision Transformer)</option>
            <option value="generic_llm">Generic LLM</option>
          </select>
        </div>

        {newModel.model_type && renderTypeSpecificFields()}

        <div className="grid grid-cols-1 md:grid-cols-3 gap-2 mb-3">
          <div>
            <label className="block text-xs text-neutral-400 mb-1">
              Labels to Exclude (comma-separated)
            </label>
            <input
              type="text"
              value={newModel.label_exclude}
              onChange={(e) =>
                setNewModel({ ...newModel, label_exclude: e.target.value })
              }
              placeholder="nsfw, adult"
              className={inputCls()}
            />
          </div>
          <div>
            <label className="block text-xs text-neutral-400 mb-1">
              Min Confidence (0-1)
            </label>
            <input
              type="number"
              step="0.01"
              min="0"
              max="1"
              value={newModel.min_confidence}
              onChange={(e) =>
                setNewModel({ ...newModel, min_confidence: e.target.value })
              }
              placeholder="0.4"
              className={inputCls()}
            />
          </div>
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

type Tab = "flag-terms" | "delete-terms" | "deletions" | "models";

export default function LabelManagement({
  pub,
  url,
}: {
  pub: EventPublisher;
  url: string;
}) {
  const [tab, setTab] = useState<Tab>("flag-terms");

  const tabCls = (t: Tab) =>
    `px-3 py-1 text-xs rounded-sm ${
      tab === t
        ? "bg-blue-900/30 text-blue-400 border border-blue-800"
        : "bg-neutral-800 text-neutral-400 border border-neutral-700 hover:text-neutral-200"
    }`;

  return (
    <div className="space-y-4">
      <div className="flex gap-2 border-b border-neutral-800 pb-2">
        <button
          onClick={() => setTab("flag-terms")}
          className={tabCls("flag-terms")}
        >
          Flag Terms
        </button>
        <button
          onClick={() => setTab("delete-terms")}
          className={tabCls("delete-terms")}
        >
          Delete terms
        </button>
        <button
          onClick={() => setTab("deletions")}
          className={tabCls("deletions")}
        >
          Label deletions
        </button>
        <button onClick={() => setTab("models")} className={tabCls("models")}>
          Label Models
        </button>
      </div>

      {tab === "flag-terms" && <LabelFlagTerms pub={pub} url={url} />}
      {tab === "delete-terms" && <LabelDeleteTerms pub={pub} url={url} />}
      {tab === "deletions" && <LabelDeletions pub={pub} url={url} />}
      {tab === "models" && <LabelModels pub={pub} url={url} />}
    </div>
  );
}
