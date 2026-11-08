<script lang="ts">
  // オーバーレイのタブの並び。サイドバーがあって幅が足りるときは 2 列にし、
  // 右の列（プレビューと OBS への追加）を止めておいて、左の設定だけをスクロールする。
  // 狭いとき（OBS のドックなど）は 1 列にして、プレビューを上に置く
  import type { Snippet } from "svelte";

  let { preview, children }: { preview: Snippet; children: Snippet } = $props();
</script>

<!-- 幅は、このタブの中身の欄で決める（サイドバーの有無や開き方に関わらない） -->
<div class="@container">
  <div class="grid gap-6 group-data-vertical/tabs:@[44rem]:grid-cols-2">
    <aside
      class="flex flex-col gap-3 group-data-vertical/tabs:@[44rem]:sticky group-data-vertical/tabs:@[44rem]:top-6 group-data-vertical/tabs:@[44rem]:col-start-2 group-data-vertical/tabs:@[44rem]:row-start-1 group-data-vertical/tabs:@[44rem]:self-start"
    >
      {@render preview()}
    </aside>
    <div
      class="min-w-0 group-data-vertical/tabs:@[44rem]:col-start-1 group-data-vertical/tabs:@[44rem]:row-start-1"
    >
      {@render children()}
    </div>
  </div>
</div>
