<script lang="ts">
  import { store } from "./store.svelte.js";
  import { scheduleBehaviourSave } from "./behaviourSave.js";
  import TagInput from "$ui/TagInput.svelte";

  type Props = {
    tags: string[];
    id: string;
  };

  let { tags, id }: Props = $props();

  function addTag(t: string) {
    if (!t || tags.includes(t)) return;
    tags.push(t);
    scheduleBehaviourSave();
  }

  function removeTag(tag: string) {
    const idx = tags.indexOf(tag);
    if (idx >= 0) tags.splice(idx, 1);
    scheduleBehaviourSave();
  }
</script>

<TagInput {tags} suggestions={store.allTags} label="Tags" onadd={addTag} onremove={removeTag} />
