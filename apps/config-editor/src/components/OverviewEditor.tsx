import type { EditorCommand } from "../api/commands";
import type { RecipeDocumentDto } from "../api/types";
import { EditableTextField } from "./EditableTextField";

interface OverviewEditorProps {
  document: RecipeDocumentDto;
  onCommand: (command: EditorCommand) => Promise<boolean>;
}

export function OverviewEditor({ document, onCommand }: OverviewEditorProps) {
  const { recipe } = document;

  return (
    <div className="space-y-6 p-6">
      <section>
        <h1 className="text-xl font-semibold text-slate-950">Overview</h1>
      </section>

      <section className="grid gap-4 rounded border border-slate-200 bg-white p-4">
        <EditableTextField
          label="Name"
          value={recipe.name}
          onCommit={(value) => onCommand({ type: "SetOverviewField", field: "name", value })}
        />
        <EditableTextField
          label="Description"
          multiline
          value={recipe.description}
          onCommit={(value) => onCommand({ type: "SetOverviewField", field: "description", value })}
        />
      </section>

      <section className="grid grid-cols-3 gap-3">
        <ReadOnlyValue label="Recipe ID" value={recipe.id} />
        <ReadOnlyValue label="Schema Version" value={String(recipe.schemaVersion)} />
        <ReadOnlyValue label="Kind" value={recipe.kind} />
      </section>
    </div>
  );
}

function ReadOnlyValue({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0 rounded border border-slate-200 bg-white p-4">
      <div className="text-xs font-semibold uppercase tracking-wide text-slate-500">{label}</div>
      <div className="mt-1 truncate text-sm font-medium text-slate-950">{value}</div>
    </div>
  );
}
