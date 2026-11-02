<script lang="ts">
  import CheckIcon from "@lucide/svelte/icons/check";
  import CopyIcon from "@lucide/svelte/icons/copy";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";

  let { url }: { url: string } = $props();
  let copied = $state(false);

  async function copy() {
    await navigator.clipboard.writeText(url);
    copied = true;
    setTimeout(() => (copied = false), 1500);
  }
</script>

<div class="flex gap-2">
  <Input value={url} readonly class="font-mono text-xs" onfocus={(e) => e.currentTarget.select()} />
  <Button variant="outline" onclick={copy}>
    {#if copied}<CheckIcon />コピーしました{:else}<CopyIcon />コピー{/if}
  </Button>
</div>
