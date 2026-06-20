import { useEffect, useState } from 'react';
import { toast } from 'sonner';
import { api } from '@/api/client';
import { Card } from '@/components/Card';

export function RoutesPage() {
  const [yaml, setYaml] = useState('');
  const [path, setPath] = useState('');
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    api.configYaml()
      .then((res) => {
        setYaml(res.yaml);
        setPath(res.path);
      })
      .catch((e) => toast.error(e.message))
      .finally(() => setLoading(false));
  }, []);

  async function save() {
    setSaving(true);
    try {
      const res = await api.saveConfig(yaml);
      toast.success(`Saved — ${res.route_count} routes active`);
    } catch (e) {
      toast.error(e instanceof Error ? e.message : 'Save failed');
    } finally {
      setSaving(false);
    }
  }

  async function reload() {
    try {
      await api.reload();
      toast.success('Reloaded from disk');
      const res = await api.configYaml();
      setYaml(res.yaml);
    } catch (e) {
      toast.error(e instanceof Error ? e.message : 'Reload failed');
    }
  }

  if (loading) return <p className="text-text-secondary">Loading routes…</p>;

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 className="text-lg font-semibold">Routes configuration</h2>
          <p className="text-sm text-text-secondary font-mono">{path}</p>
        </div>
        <div className="flex gap-2">
          <button type="button" onClick={reload} className="rounded-md border border-border px-3 py-2 text-sm hover:bg-hover">
            Reload from disk
          </button>
          <button
            type="button"
            onClick={save}
            disabled={saving}
            className="rounded-md bg-primary px-4 py-2 text-sm font-medium text-bg hover:opacity-90 disabled:opacity-50"
          >
            {saving ? 'Saving…' : 'Save & apply'}
          </button>
        </div>
      </div>
      <Card className="p-0 overflow-hidden">
        <textarea
          value={yaml}
          onChange={(e) => setYaml(e.target.value)}
          spellCheck={false}
          className="mono min-h-[28rem] w-full resize-y border-0 bg-bg p-4 text-sm outline-none"
        />
      </Card>
    </div>
  );
}
