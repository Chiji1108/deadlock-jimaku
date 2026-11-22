<script lang="ts">
  // ベータのコードをもらう前の同意（DESIGN.md 17.2）。音声と文字の送り先が変わるので、変える前に聞く
  import { keyNames, providers, usesChrome } from "../../lib/engines";
  import { SITE } from "../../lib/site";
  import { panel } from "../panel.svelte";
  import ConfirmDialog from "./ConfirmDialog.svelte";
  import CreditCopy from "./CreditCopy.svelte";

  /** 選ぼうとしている作り方に、サーバーのほかに要る会社のキー（「クラウド」の途中の字幕の Qwen など） */
  const keys = $derived(panel.betaOffer ? providers(panel.betaOffer) : []);
  /** 断られても Chrome の作り方で続けられる（途中の字幕が Chrome のときだけ。core の `Store::fall_back`） */
  const continues = $derived(!panel.betaOffer || usesChrome(panel.betaOffer));
</script>

<ConfirmDialog
  bind:open={() => !!panel.betaOffer, (open) => open || (panel.betaOffer = undefined)}
  title="ベータの無料の提供を使いますか？"
  action="有効にする"
  onconfirm={panel.acceptBeta}
>
  <ul class="flex list-disc flex-col gap-2 pl-5">
    {#if keys.length}
      <li>
        ベータ期間中は、デドロ字幕のサーバーに頼む分を無料で使えます。この作り方には、ほかに{keyNames(
          keys,
        )}が要ります。
      </li>
    {:else}
      <li>ベータ期間中は、無料で使えます。API キーは要りません。</li>
    {/if}
    <li>
      1 日に使える量には上限があり、提供は予告なく止まったり終わったりすることがあります。{continues
        ? "そのときも字幕は止まらず、Chrome の作り方で続きます。"
        : "そのときは、同じモデルの自分の API キーがそろっていればそれで続き、なければ字幕が止まります。"}
    </li>
    <li>
      字幕を出している間、話している間の音声と字幕の文字が、デドロ字幕のサーバーを通って、モデルの会社に送られます。サーバーは音声と文字を保存しません（<a
        class="underline"
        href={`${SITE}/privacy`}
        target="_blank"
        rel="noreferrer">プライバシーポリシー</a
      >）。
    </li>
  </ul>
  <div class="mt-2 flex flex-col gap-2 rounded-lg border p-3">
    <p>
      お願い:
      よければ、配信の概要欄などに、使ったツールとして載せてください。無料の提供を続ける助けになります（載せなくても使えます）。
    </p>
    <CreditCopy />
  </div>
</ConfirmDialog>
