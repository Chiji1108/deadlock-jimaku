<script lang="ts" module>
  /** 新しいコードの初期値。サーバーの既定（`DEFAULT_DAILY_LIMIT_USD`）は NULL のコードのためのもの */
  export const DEFAULT_LIMIT = 60;
  export const validLimit = (limit: number | null): limit is number => limit !== null && limit > 0;
</script>

<script lang="ts">
  // ギフトコードの 1 日の上限（DESIGN.md 17.1）。欄を空にすると null
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
    class="w-40"
    bind:value={() => value ?? undefined, (next) => (value = next ?? null)}
  />
  <Field.Description>
    {#if validLimit(value)}
      上限に達すると字幕が止まります。{LIMIT_RESET}
    {:else}
      <span class="text-destructive">0 より大きい額を入れてください</span>
    {/if}
  </Field.Description>
</Field.Field>
