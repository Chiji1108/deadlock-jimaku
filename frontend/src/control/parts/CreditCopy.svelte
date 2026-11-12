<script lang="ts">
  // 配信の概要欄などに載せてもらう 1 行と、コピーのボタン
  import CheckIcon from "@lucide/svelte/icons/check";
  import CopyIcon from "@lucide/svelte/icons/copy";
  import { Button } from "$lib/components/ui/button";
  import { CREDIT } from "../../lib/site";

  let copied = $state(false);

  async function copy() {
    await navigator.clipboard.writeText(CREDIT);
    copied = true;
    setTimeout(() => (copied = false), 1500);
  }
</script>

<div class="flex items-center gap-1">
  <code class="min-w-0 flex-1 rounded bg-muted px-2 py-1 text-xs wrap-anywhere text-foreground">
    {CREDIT}
  </code>
  <Button variant="ghost" size="icon-sm" aria-label="概要欄に載せる文をコピー" onclick={copy}>
    {#if copied}<CheckIcon />{:else}<CopyIcon />{/if}
  </Button>
</div>
