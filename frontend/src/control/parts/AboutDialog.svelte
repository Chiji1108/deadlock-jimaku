<script lang="ts">
  import { Button } from "$lib/components/ui/button";
  import * as Dialog from "$lib/components/ui/dialog";
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
</script>

<Dialog.Root>
  <Dialog.Trigger>
    {#snippet child({ props })}
      <Button variant="link" size="sm" class="h-auto p-0 text-xs text-muted-foreground" {...props}>
        v{panel.view?.version}
      </Button>
    {/snippet}
  </Dialog.Trigger>
  <Dialog.Content class="sm:max-w-sm">
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
    <p class="text-xs text-muted-foreground">
      Valve の公式のものではありません。ゲームのデータとアイコンは deadlock-api から取得しています
    </p>
  </Dialog.Content>
</Dialog.Root>
