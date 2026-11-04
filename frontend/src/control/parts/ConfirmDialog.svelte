<script lang="ts">
  import type { Snippet } from "svelte";
  import { Button } from "$lib/components/ui/button";
  import * as Dialog from "$lib/components/ui/dialog";

  // 取り返しのつかない操作や、確かめてから進めたい操作の確認。処理中は閉じられず、成功したら閉じる
  let {
    open = $bindable(),
    title,
    action,
    destructive = false,
    disabled = false,
    onconfirm,
    children,
  }: {
    open: boolean;
    title: string;
    /** 進めるボタンの文 */
    action: string;
    destructive?: boolean;
    disabled?: boolean;
    /** 成功したら true */
    onconfirm: () => Promise<boolean>;
    children: Snippet;
  } = $props();

  let busy = $state(false);

  async function confirm() {
    busy = true;
    try {
      open = !(await onconfirm());
    } finally {
      busy = false;
    }
  }
</script>

<Dialog.Root {open} onOpenChange={(next) => !busy && (open = next)}>
  <Dialog.Content class="sm:max-w-md">
    <Dialog.Header>
      <Dialog.Title>{title}</Dialog.Title>
    </Dialog.Header>
    <div class="flex flex-col gap-2 text-sm text-muted-foreground">{@render children()}</div>
    <Dialog.Footer>
      <Dialog.Close>
        {#snippet child({ props })}
          <Button variant="outline" disabled={busy} {...props}>閉じる</Button>
        {/snippet}
      </Dialog.Close>
      <Button
        variant={destructive ? "destructive" : "default"}
        disabled={busy || disabled}
        onclick={confirm}
      >
        {action}
      </Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
