<script lang="ts">
  import { cn, type WithElementRef } from "$lib/utils.js";
  import type { HTMLThAttributes } from "svelte/elements";

  let {
    ref = $bindable(null),
    class: className,
    children,
    ...restProps
  }: WithElementRef<HTMLThAttributes> = $props();
</script>

<th bind:this={ref} data-slot="table-head" class={cn("py-3 text-left font-normal whitespace-nowrap", className)} {...restProps}>
  {@render children?.()}
</th>
