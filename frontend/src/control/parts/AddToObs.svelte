<script lang="ts">
  // OBS に追加する: ボタンを OBS にドラッグするとブラウザソースができる。うまくいかなければ URL をコピーして貼る。
  // プレビューのそばに置くので小さくまとめ、長い説明は「うまくいかないとき」の窓に入れる。
  // ドラッグする部分は窓の中に入れない（窓は開いたときだけあり、Windows ではドラッグの扱いをアプリで止めている）
  // URL の layer-name / layer-width / layer-height は、ドラッグで作るソースの名前と大きさ（OBS 25 以降）
  import CheckIcon from "@lucide/svelte/icons/check";
  import CopyIcon from "@lucide/svelte/icons/copy";
  import GripIcon from "@lucide/svelte/icons/grip-vertical";
  import InfoIcon from "@lucide/svelte/icons/info";
  import { toast } from "svelte-sonner";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";
  import * as Popover from "$lib/components/ui/popover";

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

<div class="flex flex-col gap-2 rounded-lg border bg-card p-3">
  <div
    draggable="true"
    ondragstart={drag}
    role="note"
    class="flex cursor-grab items-center justify-center gap-2 rounded-md border-2 border-dashed border-primary/50 bg-primary/5 px-3 py-2.5 text-sm font-medium select-none active:cursor-grabbing"
  >
    <GripIcon class="size-4 shrink-0" />ここを OBS にドラッグして追加
  </div>
  <div class="flex flex-wrap items-center justify-between gap-x-2">
    <Button variant="ghost" size="xs" class="text-muted-foreground" onclick={copy}>
      {#if copied}<CheckIcon />コピーしました{:else}<CopyIcon />URL をコピー{/if}
    </Button>
    <Popover.Root>
      <Popover.Trigger>
        {#snippet child({ props })}
          <Button variant="ghost" size="xs" class="text-muted-foreground" {...props}>
            <InfoIcon />うまくいかないとき
          </Button>
        {/snippet}
      </Popover.Trigger>
      <Popover.Content align="end" class="w-80 text-muted-foreground">
        <p>
          OBS のプレビューかソースの一覧に落とすと、大きさを合わせたブラウザソース（{name}）ができます。OBS
          を管理者として起動していると使えません。
        </p>
        <p>
          そのときは、この URL
          をソースの「＋」→「ブラウザ」に貼り付け、幅と高さをプレビューの下の大きさにしてください。{hint}
        </p>
        <Input
          value={url}
          readonly
          class="font-mono text-xs"
          onfocus={(e) => e.currentTarget.select()}
        />
      </Popover.Content>
    </Popover.Root>
  </div>
</div>
