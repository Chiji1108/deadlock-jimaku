<script lang="ts">
  // 更新したあとの最初の起動で、何が変わったかを出す（CHANGELOG.md）。閉じたら、今の版まで見たことにする
  import { Button } from "$lib/components/ui/button";
  import * as Dialog from "$lib/components/ui/dialog";
  import { RELEASES, releaseOf, unseen } from "../../lib/changelog";
  import { panel } from "../panel.svelte";
  import ReleaseNotes from "./ReleaseNotes.svelte";

  const version = $derived(panel.view?.version);
  const releases = $derived(version ? unseen(RELEASES, panel.settings?.notesSeen, version) : []);

  function close() {
    if (version) void panel.update({ notesSeen: releaseOf(version) });
  }
</script>

<Dialog.Root open={releases.length > 0} onOpenChange={(open) => open || close()}>
  <Dialog.Content class="max-h-[85vh] overflow-y-auto sm:max-w-md">
    <Dialog.Header>
      <Dialog.Title>更新しました</Dialog.Title>
      <Dialog.Description>v{version} で変わったこと</Dialog.Description>
    </Dialog.Header>
    <ReleaseNotes {releases} />
    <Dialog.Footer>
      <Dialog.Close>
        {#snippet child({ props })}
          <Button {...props}>閉じる</Button>
        {/snippet}
      </Dialog.Close>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
