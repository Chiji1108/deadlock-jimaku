<script lang="ts">
  import * as Dialog from "$lib/components/ui/dialog";
  import { Button } from "$lib/components/ui/button";

  const ROWS = [
    { label: "文字起こし", on: "Deadlock の用語を聞き取りやすくする", off: "普通の配信として扱う" },
    { label: "強調表示", on: "ヒーロー・スキル・アイテムに色とアイコン", off: "なし" },
    { label: "アイテム説明", on: "出る", off: "出ない" },
    { label: "翻訳", on: "ゲーム内の正式な名前にする", off: "普通の文として訳す" },
  ];
</script>

<Dialog.Root>
  <Dialog.Trigger>
    {#snippet child({ props })}
      <Button {...props} variant="link" size="sm" class="h-auto p-0">詳しく</Button>
    {/snippet}
  </Dialog.Trigger>
  <Dialog.Content class="sm:max-w-xl">
    <Dialog.Header>
      <Dialog.Title>デッドロックモード</Dialog.Title>
      <Dialog.Description>
        雑談の配信や、ほかのゲームの配信ではオフにしてください。切り替えると、字幕は始め直します
      </Dialog.Description>
    </Dialog.Header>
    <table class="w-full text-sm">
      <thead>
        <tr class="border-b text-left">
          <th scope="col"><span class="sr-only">項目</span></th>
          <th scope="col" class="py-2 pr-3 font-medium">オン</th>
          <th scope="col" class="py-2 font-medium">オフ</th>
        </tr>
      </thead>
      <tbody>
        {#each ROWS as row (row.label)}
          <tr class="border-b last:border-0">
            <th
              scope="row"
              class="py-2 pr-3 text-left font-normal whitespace-nowrap text-muted-foreground"
            >
              {row.label}
            </th>
            <td class="py-2 pr-3">{row.on}</td>
            <td class="py-2">{row.off}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  </Dialog.Content>
</Dialog.Root>
