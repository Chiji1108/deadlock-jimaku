<script lang="ts">
  // 更新したあとの最初の起動で、何が変わったかを出す（CHANGELOG.md）。閉じたら、今の版まで見たことにする
  import { Button } from "$lib/components/ui/button";
  import * as Dialog from "$lib/components/ui/dialog";
  import { RELEASES, releaseOf, unseen } from "../../lib/changelog";
  import { BETA, modeOf } from "../../lib/engines";
  import { panel } from "../panel.svelte";
  import ReleaseNotes from "./ReleaseNotes.svelte";

  const version = $derived(panel.view?.version);
  const releases = $derived(version ? unseen(RELEASES, panel.settings?.notesSeen, version) : []);

  /** Chrome のモードの人には、ここから「おすすめ」に切り替えられるようにする（更新の直後がいちばん試しやすい） */
  const suggest = $derived(!!panel.settings && modeOf(panel.settings.engines)?.name === "chrome");

  async function close() {
    if (version) await panel.update({ notesSeen: releaseOf(version) });
  }

  async function recommend() {
    await close();
    await panel.chooseMode(BETA);
  }
</script>

<Dialog.Root open={releases.length > 0} onOpenChange={(open) => open || void close()}>
  <Dialog.Content class="max-h-[85vh] overflow-y-auto sm:max-w-md">
    <Dialog.Header>
      <Dialog.Title>更新しました</Dialog.Title>
      <Dialog.Description>v{version} で変わったこと</Dialog.Description>
    </Dialog.Header>
    <ReleaseNotes {releases} />
    <a
      class="text-sm underline"
      href={`https://github.com/Chiji1108/deadlock-jimaku/releases/tag/v${version}`}
      target="_blank"
      rel="noreferrer">詳しくはこちら</a
    >
    <Dialog.Footer>
      <Dialog.Close>
        {#snippet child({ props })}
          <Button variant={suggest ? "outline" : "default"} {...props}>閉じる</Button>
        {/snippet}
      </Dialog.Close>
      {#if suggest}
        <Button onclick={recommend}>おすすめにする</Button>
      {/if}
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
