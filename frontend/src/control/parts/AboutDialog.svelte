<script lang="ts">
  import { Button } from "$lib/components/ui/button";
  import * as Dialog from "$lib/components/ui/dialog";
  import fonts from "../../../../notices/fonts.txt?raw";
  import lindera from "../../../../notices/lindera.txt?raw";
  import mecabIpadic from "../../../../notices/mecab-ipadic.txt?raw";
  import { panel } from "../panel.svelte";

  // アプリの情報。左下の版から開く（毎日見るものではないので、サイドバーの項目にしない）
  const SITE = "https://jimaku.deadlock.jp";
  const LINKS = [
    { href: `${SITE}/#faq`, label: "使い方・よくある質問" },
    { href: "https://discord.gg/QdVFzMBBD2", label: "Discord（不具合の報告・要望）" },
    { href: `${SITE}/terms`, label: "利用規約" },
    { href: `${SITE}/privacy`, label: "プライバシーポリシー" },
    {
      href: "https://github.com/Chiji1108/deadlock-jimaku/blob/main/LICENSE.md",
      label: "ライセンス",
    },
  ];
  // ふりがなと辞書の一致に使い、アプリに組み込んでいるもの。利用規約（web）と同じ文を出す
  const NOTICES = [
    { name: "mecab-ipadic", text: mecabIpadic },
    { name: "Lindera", text: lindera },
    // アプリには含まず、選んだ人だけがダウンロードする
    { name: "おすすめのフォント", text: fonts },
  ];
  // 全文は 300KB あるので、開いたときに読み込む（毎日開くページのバンドルに入れない）
  let thirdParty = $state<Promise<string>>();
</script>

<Dialog.Root>
  <Dialog.Trigger>
    {#snippet child({ props })}
      <Button variant="link" size="sm" class="h-auto p-0 text-xs text-muted-foreground" {...props}>
        v{panel.view?.version}
      </Button>
    {/snippet}
  </Dialog.Trigger>
  <Dialog.Content class="max-h-[85vh] overflow-y-auto sm:max-w-sm">
    <Dialog.Header>
      <Dialog.Title>デドロ字幕</Dialog.Title>
      <Dialog.Description>v{panel.view?.version} ・ 作者: ミリちゃんねる</Dialog.Description>
    </Dialog.Header>
    <ul class="flex flex-col gap-2 text-sm">
      {#each LINKS as link (link.href)}
        <li>
          <a class="underline" href={link.href} target="_blank" rel="noreferrer">{link.label}</a>
        </li>
      {/each}
    </ul>
    <div class="flex flex-col gap-1 text-xs">
      {#each NOTICES as notice (notice.name)}
        <details>
          <summary class="cursor-pointer">{notice.name} のライセンス</summary>
          <pre
            class="mt-1 max-h-48 overflow-y-auto font-sans whitespace-pre-wrap">{notice.text}</pre>
        </details>
      {/each}
      <details
        ontoggle={() =>
          (thirdParty ??= import("../../../../notices/third-party.txt?raw").then(
            (module) => module.default,
          ))}
      >
        <summary class="cursor-pointer">第三者のライセンス</summary>
        {#await thirdParty then text}
          <pre
            class="mt-1 max-h-64 overflow-y-auto font-sans wrap-anywhere whitespace-pre-wrap">{text}</pre>
        {:catch}
          <p class="mt-1 text-muted-foreground">読み込めませんでした</p>
        {/await}
      </details>
    </div>
    <p class="text-xs text-muted-foreground">
      Valve の公式のものではありません。ゲームのデータとアイコンは deadlock-api から取得しています
    </p>
  </Dialog.Content>
</Dialog.Root>
