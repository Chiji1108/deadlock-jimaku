<script lang="ts" module>
  export const HEX = /^#[0-9a-fA-F]{6}$/;
</script>

<script lang="ts">
  import { Input } from "$lib/components/ui/input";

  // 色の候補、OS の色選び、#rrggbb の入力を並べる。入力途中（形式が合わない間）は色選びに前の色を出す
  let {
    id,
    value = $bindable(),
    fallback,
    swatches = [],
  }: {
    id: string;
    value: string;
    fallback: string;
    swatches?: { hex: string; label: string }[];
  } = $props();
</script>

<div class="flex flex-wrap items-center gap-2">
  {#each swatches as c (c.hex)}
    <button
      type="button"
      class="size-7 rounded-full border-2 ring-offset-2 ring-offset-background transition {value.toLowerCase() ===
      c.hex
        ? 'border-primary ring-2 ring-primary'
        : 'border-border'}"
      style:background={c.hex}
      aria-label={c.label}
      aria-pressed={value.toLowerCase() === c.hex}
      onclick={() => (value = c.hex)}
    ></button>
  {/each}
  <input
    type="color"
    class="size-9 cursor-pointer rounded-md border-0 bg-transparent p-0"
    aria-label="色を選ぶ"
    value={HEX.test(value) ? value : fallback}
    oninput={(e) => (value = e.currentTarget.value)}
  />
  <Input {id} class="w-28 font-mono" bind:value />
</div>
