<script lang="ts">
  // OBS に追加する: ボタンを OBS にドラッグするとブラウザソースができる。うまくいかなければ URL をコピーして貼る。
  // URL の layer-name / layer-width / layer-height は、ドラッグで作るソースの名前と大きさ（OBS 25 以降）
  import CheckIcon from "@lucide/svelte/icons/check";
  import CopyIcon from "@lucide/svelte/icons/copy";
  import GripIcon from "@lucide/svelte/icons/grip-vertical";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";

  let { url, name, size }: { url: string; name: string; size: [number, number] } = $props();
  let copied = $state(false);

  const dragUrl = $derived.by(() => {
    const u = new URL(url);
    // 空白は + になり、OBS がそのまま + として読むことがあるので使わない
    u.searchParams.set("layer-name", `デドロ字幕（${name}）`);
    u.searchParams.set("layer-width", String(size[0]));
    u.searchParams.set("layer-height", String(size[1]));
    return u.toString();
  });

  function drag(e: DragEvent) {
    e.dataTransfer?.setData("text/uri-list", dragUrl);
    e.dataTransfer?.setData("text/plain", dragUrl);
  }

  async function copy() {
    await navigator.clipboard.writeText(url);
    copied = true;
    setTimeout(() => (copied = false), 1500);
  }
</script>

<div class="flex flex-col gap-3">
  <a
    href={dragUrl}
    draggable="true"
    ondragstart={drag}
    onclick={(e) => e.preventDefault()}
    class="flex cursor-grab items-center justify-center gap-2 rounded-md border-2 border-dashed border-primary/50 bg-primary/5 px-4 py-3 text-sm font-medium active:cursor-grabbing"
  >
    <GripIcon class="size-4" />このボタンを OBS にドラッグして追加（{name}）
  </a>
  <div class="flex gap-2">
    <Input
      value={url}
      readonly
      class="font-mono text-xs"
      onfocus={(e) => e.currentTarget.select()}
    />
    <Button variant="outline" onclick={copy}>
      {#if copied}<CheckIcon />コピーしました{:else}<CopyIcon />コピー{/if}
    </Button>
  </div>
</div>
