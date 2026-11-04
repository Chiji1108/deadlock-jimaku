<script lang="ts">
  // OpenAI の API キーの作り方。OpenAI を使ったことのない配信者向けに、支払いの登録まで順に書く。
  // キーがまだないときは開いておく（開くかどうかは最初に決めるだけ。保存したとたんに閉じない）
  import { untrack } from "svelte";
  import * as Card from "$lib/components/ui/card";

  let { open }: { open: boolean } = $props();
  const initiallyOpen = untrack(() => open);

  const steps: { title: string; body: string; link?: { href: string; label: string } }[] = [
    {
      title: "OpenAI の開発者向けサイトに登録する",
      body: "メールアドレスか Google アカウントで登録します。ChatGPT のアカウントがあれば、それでログインできます。ChatGPT の有料プランとは別の支払いになります",
      link: { href: "https://platform.openai.com/signup", label: "登録ページを開く" },
    },
    {
      title: "クレジットカードを登録して、前払いでチャージする",
      body: "設定の「Billing」で「Add payment method」からカードを登録し、「Add to credit balance」で 5〜10 ドルほどチャージします。使った分だけ残高から引かれます。使いすぎが心配なら、自動チャージ（Auto recharge）はオフのままにしてください",
      link: {
        href: "https://platform.openai.com/settings/organization/billing/overview",
        label: "Billing を開く",
      },
    },
    {
      title: "API キーを作る",
      body: "「API keys」で「Create new secret key」を押し、名前（例: デドロ字幕）を付けて作ります。キーは作ったときに一度しか表示されないので、すぐにコピーしてください",
      link: { href: "https://platform.openai.com/api-keys", label: "API keys を開く" },
    },
    {
      title: "下の欄に貼り付けて保存する",
      body: "キーはこの PC の資格情報ストアに保存され、画面には表示しません。キーは他の人に見せないでください（使われると、あなたの残高から引かれます）",
    },
  ];
</script>

<Card.Root>
  <details open={initiallyOpen} class="group">
    <!-- 標準の三角は消して（WebKit は ::-webkit-details-marker）、自前の › だけにする -->
    <summary
      class="cursor-pointer list-none px-6 font-semibold select-none [&::-webkit-details-marker]:hidden"
    >
      <span aria-hidden="true" class="mr-2 inline-block transition group-open:rotate-90">›</span
      >OpenAI の API キーの作り方
    </summary>
    <Card.Content class="mt-5 flex flex-col gap-5">
      <p class="text-sm text-muted-foreground">
        文字起こしと翻訳は OpenAI に頼むので、OpenAI の API
        キーが必要です。料金は使った分だけで、話していない間はかかりません。話す時間が半分くらいの配信なら
        1 時間あたり約 1 ドル（翻訳 1
        言語・ふりがなあり）が目安です。使った分の目安は「料金」タブに出ます
      </p>
      <ol class="flex flex-col gap-4">
        {#each steps as step, i (step.title)}
          <li class="flex gap-3">
            <span
              class="flex size-6 shrink-0 items-center justify-center rounded-full bg-primary text-xs font-semibold text-primary-foreground"
              >{i + 1}</span
            >
            <div class="flex flex-col gap-1">
              <p class="text-sm font-medium">{step.title}</p>
              <p class="text-sm text-muted-foreground">{step.body}</p>
              {#if step.link}
                <a
                  class="w-fit text-sm underline"
                  href={step.link.href}
                  target="_blank"
                  rel="noreferrer">{step.link.label}</a
                >
              {/if}
            </div>
          </li>
        {/each}
      </ol>
    </Card.Content>
  </details>
</Card.Root>
