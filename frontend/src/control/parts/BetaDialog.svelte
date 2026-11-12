<script lang="ts">
  // ベータのコードをもらう前の同意（DESIGN.md 17.2）。音声と文字の送り先が変わるので、変える前に聞く
  import { SITE } from "../../lib/site";
  import { panel } from "../panel.svelte";
  import ConfirmDialog from "./ConfirmDialog.svelte";
  import CreditCopy from "./CreditCopy.svelte";
</script>

<ConfirmDialog
  bind:open={() => !!panel.betaOffer, (open) => open || (panel.betaOffer = undefined)}
  title="ベータの無料の提供を使いますか？"
  action="有効にする"
  onconfirm={panel.acceptBeta}
>
  <ul class="flex list-disc flex-col gap-2 pl-5">
    <li>ベータの間は、API の料金を開発者が負担するので、無料で使えます。</li>
    <li>
      1 日に使える量には上限があります。提供は、予告なく止まったり終わったりすることがあります。
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
