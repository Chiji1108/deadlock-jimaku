<script lang="ts">
  // OBS に追加する: ボタンを OBS にドラッグするとブラウザソースができる。うまくいかなければ URL をコピーして貼る。
  // URL の layer-name / layer-width / layer-height は、ドラッグで作るソースの名前と大きさ（OBS 25 以降）
  import CheckIcon from "@lucide/svelte/icons/check";
  import CopyIcon from "@lucide/svelte/icons/copy";
  import GripIcon from "@lucide/svelte/icons/grip-vertical";
  import { toast } from "svelte-sonner";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";

  let {
    url,
    name,
    size,
    hint,
  }: {
    url: string;
    name: string;
    size: [number, number];
    /** 追加したあとの大きさの変え方 */
    hint: string;
  } = $props();
  let copied = $state(false);
  let reset: ReturnType<typeof setTimeout> | undefined;

  const dragUrl = $derived.by(() => {
    const u = new URL(url);
    // 空白は + になり、OBS がそのまま + として読むことがあるので使わない
    u.searchParams.set("layer-name", `デドロ字幕（${name}）`);
    u.searchParams.set("layer-width", String(size[0]));
    u.searchParams.set("layer-height", String(size[1]));
    return u.toString();
  });

  // リンクではなくドラッグだけの部品にし、渡すのも URL だけにする
  // （ブラウザが付けるショートカットのファイルなどを OBS が先に読むと、ソースができない）
  function drag(e: DragEvent) {
    if (!e.dataTransfer) return;
    e.dataTransfer.clearData();
    e.dataTransfer.setData("text/uri-list", dragUrl);
    e.dataTransfer.setData("text/plain", dragUrl);
    e.dataTransfer.effectAllowed = "copy";
  }

  async function copy() {
    try {
      await navigator.clipboard.writeText(url);
    } catch {
      toast.error("コピーできませんでした。URL を選んでコピーしてください");
      return;
    }
    copied = true;
    clearTimeout(reset);
    reset = setTimeout(() => (copied = false), 1500);
  }
</script>

<div class="flex flex-col gap-3">
  <p class="text-sm font-medium">OBS に追加する</p>
  <div
    draggable="true"
    ondragstart={drag}
    role="note"
    class="flex cursor-grab items-center justify-center gap-2 rounded-md border-2 border-dashed border-primary/50 bg-primary/5 px-4 py-3 text-sm font-medium select-none active:cursor-grabbing"
  >
    <GripIcon class="size-4" />ここを OBS にドラッグして追加（{name}）
  </div>
  <p class="text-sm text-muted-foreground">
    OBS のプレビューかソースの一覧に落とすと、大きさを合わせたブラウザソースができます（OBS
    を管理者として起動していると使えません）。うまくいかないときは、下の URL
    をコピーしてソースの「＋」→「ブラウザ」に貼り付け、幅 {size[0]}・高さ {size[1]} にしてください。{hint}
  </p>
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
