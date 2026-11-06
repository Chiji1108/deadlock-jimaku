<script lang="ts">
  // ギフトコードの 1 日の上限。空欄（null）は既定の額（DESIGN.md 17.1）
  import * as Field from "$lib/components/ui/field";
  import { Input } from "$lib/components/ui/input";

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
    上限に達すると、その日は字幕が止まります。日本時間の朝 9 時（UTC の 0 時）に戻ります
  </Field.Description>
</Field.Field>
