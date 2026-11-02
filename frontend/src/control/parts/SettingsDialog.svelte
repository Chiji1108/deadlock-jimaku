<script lang="ts">
  import SlidersHorizontalIcon from "@lucide/svelte/icons/sliders-horizontal";
  import type { Snippet } from "svelte";
  import { Button } from "$lib/components/ui/button";
  import * as Dialog from "$lib/components/ui/dialog";

  // 並べた選択肢で足りない人向けに、値を細かく決めるダイアログの外枠。
  // 開くたびに onopen で今の設定から始め、保存できたら閉じる
  let {
    trigger,
    title,
    description,
    valid,
    onopen,
    onsave,
    children,
  }: {
    trigger: string;
    title: string;
    description?: string;
    valid: boolean;
    onopen: () => void;
    /** 保存できたら true */
    onsave: () => Promise<boolean>;
    children: Snippet;
  } = $props();

  let open = $state(false);

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    if (await onsave()) open = false;
  }
</script>

<Dialog.Root bind:open onOpenChange={(o) => o && onopen()}>
  <Dialog.Trigger>
    {#snippet child({ props })}
      <Button variant="ghost" size="sm" {...props}><SlidersHorizontalIcon />{trigger}</Button>
    {/snippet}
  </Dialog.Trigger>
  <Dialog.Content class="sm:max-w-md">
    <form onsubmit={submit} class="flex flex-col gap-6">
      <Dialog.Header>
        <Dialog.Title>{title}</Dialog.Title>
        {#if description}<Dialog.Description>{description}</Dialog.Description>{/if}
      </Dialog.Header>
      {@render children()}
      <Dialog.Footer>
        <Dialog.Close>
          {#snippet child({ props })}
            <Button variant="outline" {...props}>やめる</Button>
          {/snippet}
        </Dialog.Close>
        <Button type="submit" disabled={!valid}>保存</Button>
      </Dialog.Footer>
    </form>
  </Dialog.Content>
</Dialog.Root>
