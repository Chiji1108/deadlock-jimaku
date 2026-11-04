<script lang="ts">
  // アイテム説明の節の見本
  import { onMount } from "svelte";
  import { defineCards, type Card } from "../lib/cards";

  let { card }: { card: Card } = $props();
  let ready = $state(false);

  onMount(() => void defineCards().then(() => (ready = true)));
</script>

<dl-provider language="japanese" role="img" aria-label="アイテム説明のカード（ディスペルマジック）">
  {#if ready}
    <dl-item-tooltip
      itemData={card.item}
      componentItemsData={card.components}
      parentItemsData={card.parents}
    ></dl-item-tooltip>
  {/if}
</dl-provider>
