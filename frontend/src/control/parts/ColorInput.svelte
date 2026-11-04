<script lang="ts">
  import { Input } from "$lib/components/ui/input";

  // OS の色選びと #rrggbb の入力。選んでいる間も字幕に反映するが、送りすぎないよう少し待つ
  const HEX = /^#[0-9a-fA-F]{6}$/;
  const DEBOUNCE_MS = 150;

  let { id, value, onchange }: { id: string; value: string; onchange: (hex: string) => void } =
    $props();

  let edited = $state<string | undefined>();
  const text = $derived(edited ?? value);
  let timer: ReturnType<typeof setTimeout> | undefined;

  function input(next: string) {
    edited = next;
    clearTimeout(timer);
    if (!HEX.test(next)) return;
    timer = setTimeout(() => {
      onchange(next.toLowerCase());
      edited = undefined;
    }, DEBOUNCE_MS);
  }
</script>

<div class="flex items-center gap-2">
  <input
    type="color"
    class="size-9 cursor-pointer rounded-md border-0 bg-transparent p-0"
    aria-label="色を選ぶ"
    value={HEX.test(text) ? text : value}
    oninput={(e) => input(e.currentTarget.value)}
  />
  <Input
    {id}
    class="w-28 font-mono"
    value={text}
    oninput={(e) => input(e.currentTarget.value)}
    onblur={() => (edited = undefined)}
  />
</div>
