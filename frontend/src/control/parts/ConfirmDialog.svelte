<script lang="ts">
  import type { Snippet } from "svelte";
  import { Button } from "$lib/components/ui/button";
  import * as Dialog from "$lib/components/ui/dialog";

  // 取り返しのつかない操作や、確かめてから進めたい操作の確認。やる処理が終わるまで閉じない
  let {
    open = $bindable(),
    title,
    action,
    destructive = false,
    onconfirm,
    children,
  }: {
    open: boolean;
    title: string;
    /** 進めるボタンの文 */
    action: string;
    destructive?: boolean;
    onconfirm: () => Promise<unknown>;
    children: Snippet;
  } = $props();

  let busy = $state(false);

  async function confirm() {
    busy = true;
    await onconfirm();
    busy = false;
    open = false;
  }
</script>

<Dialog.Root bind:open>
  <Dialog.Content class="sm:max-w-md">
    <Dialog.Header>
      <Dialog.Title>{title}</Dialog.Title>
    </Dialog.Header>
    <div class="flex flex-col gap-2 text-sm text-muted-foreground">{@render children()}</div>
    <Dialog.Footer>
      <Dialog.Close>
        {#snippet child({ props })}
          <Button variant="outline" {...props}>閉じる</Button>
        {/snippet}
      </Dialog.Close>
      <Button variant={destructive ? "destructive" : "default"} disabled={busy} onclick={confirm}>
        {action}
      </Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
