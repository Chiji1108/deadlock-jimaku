<script lang="ts">
  // 版ごとの更新の内容。1 つの版だけなら、版の見出しは出さない
  import type { Release } from "../../lib/changelog";

  let { releases }: { releases: Release[] } = $props();
</script>

<div class="flex flex-col gap-4 text-sm">
  {#each releases as release (release.version)}
    <section class="flex flex-col gap-2">
      {#if releases.length > 1}
        <h3 class="font-medium tabular-nums">v{release.version}</h3>
      {/if}
      <ul class="flex list-disc flex-col gap-2 pl-5 text-muted-foreground">
        {#each release.items as item (item)}
          <li>{item}</li>
        {/each}
      </ul>
    </section>
  {/each}
</div>
