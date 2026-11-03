<script lang="ts">
  // コードは配信に映っても大丈夫なよう、先頭だけ見せて伏せる。コピーと、見たいときだけの表示
  import CheckIcon from "@lucide/svelte/icons/check";
  import CopyIcon from "@lucide/svelte/icons/copy";
  import EyeIcon from "@lucide/svelte/icons/eye";
  import EyeOffIcon from "@lucide/svelte/icons/eye-off";
  import { Button } from "$lib/components/ui/button";

  let { code }: { code: string } = $props();
  let shown = $state(false);
  let copied = $state(false);

  const masked = $derived(code.replace(/(?<=^.{4}).*/, (rest) => rest.replace(/[^-]/g, "•")));

  async function copy() {
    await navigator.clipboard.writeText(code);
    copied = true;
    setTimeout(() => (copied = false), 1500);
  }
</script>

<div class="flex items-center gap-1">
  <code class="rounded bg-muted px-2 py-1 font-mono text-sm tracking-wider">
    {shown ? code : masked}
  </code>
  <Button
    variant="ghost"
    size="icon-sm"
    aria-label={shown ? "コードを隠す" : "コードを表示"}
    onclick={() => (shown = !shown)}
  >
    {#if shown}<EyeOffIcon />{:else}<EyeIcon />{/if}
  </Button>
  <Button variant="ghost" size="icon-sm" aria-label="コードをコピー" onclick={copy}>
    {#if copied}<CheckIcon />{:else}<CopyIcon />{/if}
  </Button>
</div>
