<script lang="ts" module>
  export const HEX = /^#[0-9a-fA-F]{6}$/;
</script>

<script lang="ts">
  import { Input } from "$lib/components/ui/input";

  // OS の色選びと #rrggbb の入力を並べる。入力途中（形式が合わない間）は色選びに前の色を出す
  let {
    id,
    value = $bindable(),
    fallback,
  }: { id: string; value: string; fallback: string } = $props();
</script>

<div class="flex items-center gap-2">
  <input
    type="color"
    class="size-9 cursor-pointer rounded-md border-0 bg-transparent p-0"
    aria-label="色を選ぶ"
    value={HEX.test(value) ? value : fallback}
    oninput={(e) => (value = e.currentTarget.value)}
  />
  <Input {id} class="w-28 font-mono" bind:value />
</div>
