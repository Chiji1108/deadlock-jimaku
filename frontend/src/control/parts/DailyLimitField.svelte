<script lang="ts">
  // ギフトコードの 1 日の上限。空欄（null）は既定の額（DESIGN.md 17.1）
  import * as Field from "$lib/components/ui/field";
  import { Input } from "$lib/components/ui/input";
  import { LIMIT_RESET } from "../panel.svelte";

  let { id, value = $bindable() }: { id: string; value: number | null } = $props();
</script>

<Field.Field>
  <Field.Label for={id}>1 日の上限（ドル）</Field.Label>
  <Input
    {id}
    type="number"
    min="0.01"
    step="any"
    placeholder="空欄で既定の額"
    class="w-40"
    bind:value={() => value ?? undefined, (next) => (value = next ?? null)}
  />
  <Field.Description>
    上限に達すると字幕が止まります。{LIMIT_RESET}
  </Field.Description>
</Field.Field>
