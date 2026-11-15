<script lang="ts">
  // 開いたときの画面。配信の間に見るものを 1 つに集める: 状態、今出ている字幕、マイクに届いている音、今のモード、言語。
  // 広いときは 2 列にして、スクロールせずに見渡せるようにする。細かい設定は、それぞれのタブへ送る（DESIGN.md 13 章）
  import ArrowRightIcon from "@lucide/svelte/icons/arrow-right";
  import PlayIcon from "@lucide/svelte/icons/play";
  import SparklesIcon from "@lucide/svelte/icons/sparkles";
  import * as Alert from "$lib/components/ui/alert";
  import { Button } from "$lib/components/ui/button";
  import * as Card from "$lib/components/ui/card";
  import ChromeWindow from "../parts/ChromeWindow.svelte";
  import LevelMeter from "../parts/LevelMeter.svelte";
  import ModeBadges from "../parts/ModeBadges.svelte";
  import Onboarding from "../parts/Onboarding.svelte";
  import OverlayPreview from "../parts/OverlayPreview.svelte";
  import PlayingHero from "../parts/PlayingHero.svelte";
  import { panel, transcriptionErrorText } from "../panel.svelte";
  import { BETA, chromeFinals, modeOf, usesChrome } from "../../lib/engines";
  import { SUBTITLE_SIZE } from "../../lib/overlay-size";

  const s = $derived(panel.settings!);
  const t = $derived(s.engines);
  const mode = $derived(modeOf(t));
  const languages = $derived([...new Set([s.micLanguage, ...s.translateTo])]);
  /** 動いている間は、OBS に出ているのと同じ字幕を見せる。止まっている間は、言語ごとの見本を出しておく */
  const src = $derived(
    panel.running
      ? "/overlay/subtitles.html"
      : `/overlay/subtitles.html?preview=${languages.join(",")}`,
  );
  const status = $derived(panel.status);
  /** 状態の下に添えるひとこと。止まっているときは理由と直し方、そうでなければ今の作り方 */
  const detail = $derived.by(() => {
    if (panel.server !== "open") return "つながると、今の状態が出ます";
    if (s.transcriptionEnabled && panel.transcriptionError)
      return `${transcriptionErrorText(panel.transcriptionError)}。直したら「字幕を止める」→「字幕を始める」で始め直してください`;
    const to = s.translateTo.length ? ` → ${s.translateTo.map(panel.label).join("、")}` : "";
    return `モード: ${mode?.label ?? "カスタム"}　言語: ${panel.label(s.micLanguage)}${to}`;
  });
</script>

<div class="@container flex flex-col gap-6">
  <!-- 状態の中身。状態のひとことと開始・停止のボタンは、上の帯の 1 か所にだけ出す -->
  <Card.Root>
    <Card.Content class="gap-1">
      <p
        class={["text-sm", status.tone === "error" ? "text-destructive" : "text-muted-foreground"]}
      >
        {detail}
      </p>
      <!-- 字幕を作っていても、OBS につながっていなければ配信には出ない -->
      <p class="flex items-center gap-2 text-xs text-muted-foreground">
        <span
          class={[
            "size-2 shrink-0 rounded-full",
            panel.obsSubtitles ? "bg-emerald-500" : "bg-muted-foreground/50",
          ]}
        ></span>
        {#if panel.obsSubtitles}
          OBS に字幕がつながっています
        {:else}
          OBS に字幕がつながっていません（OBS を開いていないか、字幕を追加していません）
          <Button
            variant="link"
            size="sm"
            class="h-auto p-0 text-xs"
            onclick={() => (panel.tab = "display")}
          >
            追加する
          </Button>
        {/if}
      </p>
    </Card.Content>
  </Card.Root>

  <Onboarding open={(value) => (panel.tab = value)} />

  <!-- 列の幅は中身に押し広げさせない（狭いときに、長い名前やプレビューで横にはみ出さない） -->
  <div
    class="grid grid-cols-[minmax(0,1fr)] items-start gap-6 @[52rem]:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]"
  >
    <Card.Root>
      <Card.Header>
        <Card.Title>{panel.running ? "いま出ている字幕" : "字幕の見本"}</Card.Title>
        <Card.Description>
          {panel.running
            ? "OBS に出ているのと同じ字幕です"
            : "字幕を始めると、ここに OBS と同じ字幕が出ます"}
        </Card.Description>
        <Card.Action>
          <Button variant="outline" size="sm" onclick={() => (panel.tab = "display")}>
            見た目を変える<ArrowRightIcon />
          </Button>
        </Card.Action>
      </Card.Header>
      <Card.Content class="flex flex-col gap-4">
        <OverlayPreview {src} title="字幕" size={SUBTITLE_SIZE}>
          <Button variant="outline" size="sm" onclick={panel.demo}>
            <PlayIcon />テスト字幕を流す
          </Button>
        </OverlayPreview>
        <!-- 確定の文を Chrome が作るなら、マイクは Chrome のウィンドウが開くので、音の大きさは届かない -->
        {#if panel.running && !chromeFinals(t)}
          <div class="flex flex-col gap-2">
            <p class="text-sm font-medium">マイクに届いている音</p>
            <LevelMeter />
          </div>
        {/if}
        {#if panel.silent}
          <Alert.Root variant="destructive">
            <Alert.Title>音が届いていません</Alert.Title>
            <Alert.Description class="flex flex-col items-start gap-2">
              マイクの選択と、OS のマイクの許可（macOS: システム設定 → プライバシーとセキュリティ →
              マイク）を確認してください
              <Button variant="outline" size="sm" onclick={() => (panel.tab = "audio")}>
                マイクを選ぶ
              </Button>
            </Alert.Description>
          </Alert.Root>
        {/if}
      </Card.Content>
    </Card.Root>

    <div class="flex flex-col gap-6">
      <Card.Root>
        <Card.Header>
          <Card.Title>モード</Card.Title>
          <Card.Description>字幕をどこで作るかです</Card.Description>
          <Card.Action>
            <Button variant="outline" size="sm" onclick={() => (panel.tab = "engines")}>
              変える<ArrowRightIcon />
            </Button>
          </Card.Action>
        </Card.Header>
        <Card.Content class="flex flex-col gap-3">
          <div class="flex flex-wrap items-center gap-2 rounded-lg border p-3">
            <span class="text-sm font-medium">{mode?.label ?? "カスタム"}</span>
            {#if mode}
              <ModeBadges {mode} />
            {:else}
              <span class="text-xs text-muted-foreground">工程ごとに選んだ組み合わせ</span>
            {/if}
          </div>
          <!-- Chrome のモードの人に、「おすすめ」を知らせる。既定は Chrome のままで、選ぶのは本人（音声の送り先が変わる） -->
          {#if mode?.name === "chrome" && !s.hybridHintClosed}
            <Alert.Root>
              <SparklesIcon />
              <Alert.Title
                >「おすすめ」のモードにすると、字幕と翻訳がもっと正確になります</Alert.Title
              >
              <Alert.Description class="flex flex-col gap-2">
                <span>ベータ期間中は無料で、API キーも要りません</span>
                <span class="flex flex-wrap gap-2">
                  <Button size="sm" onclick={() => panel.chooseMode(BETA)}>おすすめにする</Button>
                  <Button
                    size="sm"
                    variant="outline"
                    onclick={() => panel.update({ hybridHintClosed: true })}>閉じる</Button
                  >
                </span>
              </Alert.Description>
            </Alert.Root>
          {/if}
          {#if usesChrome(t)}<ChromeWindow />{/if}
        </Card.Content>
      </Card.Root>
      {#if s.deadlockMode && s.micLanguage === "ja"}<PlayingHero />{/if}
      <Card.Root>
        <Card.Header>
          <Card.Title>言語</Card.Title>
          <Card.Description>
            {panel.label(s.micLanguage)}{#if s.translateTo.length}
              → {s.translateTo.map(panel.label).join("、")}{:else}（翻訳なし）{/if}
          </Card.Description>
          <Card.Action>
            <Button variant="outline" size="sm" onclick={() => (panel.tab = "audio")}>
              変える<ArrowRightIcon />
            </Button>
          </Card.Action>
        </Card.Header>
      </Card.Root>
    </div>
  </div>
</div>
