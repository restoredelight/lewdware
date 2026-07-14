<script lang="ts">
  import { CheckCircle, ExclamationTriangle, Icon, XMark } from "svelte-hero-icons";
  import { taskFeedback } from "./taskFeedback.svelte.js";
</script>

{#if taskFeedback.tone}
  <div class="status {taskFeedback.tone}" role={taskFeedback.tone === "error" ? "alert" : "status"} aria-live="polite">
    {#if taskFeedback.tone === "progress"}<span class="spinner"></span>
    {:else if taskFeedback.tone === "success"}<Icon src={CheckCircle} mini size="14px" />
    {:else}<Icon src={ExclamationTriangle} mini size="14px" />{/if}
    <span class="message">{taskFeedback.message}{#if taskFeedback.current !== null && taskFeedback.total !== null} {taskFeedback.current}/{taskFeedback.total}{/if}</span>
    {#if taskFeedback.tone === "error" || taskFeedback.tone === "warning"}<button onclick={() => taskFeedback.dismiss()} aria-label="Dismiss status"><Icon src={XMark} mini size="13px" /></button>{/if}
  </div>
{/if}

<style>
  .status { display: flex; max-width: 360px; height: 26px; padding: 0 7px; align-items: center; gap: 5px; border: 1px solid var(--ui-border); border-radius: var(--ui-radius-sm); background: var(--ui-bg); color: var(--ui-muted); font-size: 11px; }
  .message { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .success { color: var(--ui-success); border-color: var(--ui-success-border); background: var(--ui-success-bg); }
  .warning { color: var(--ui-warning); border-color: var(--ui-warning-border); background: var(--ui-warning-bg); }
  .error { color: var(--ui-danger); border-color: var(--ui-danger-border); background: var(--ui-danger-bg); }
  .spinner { width: 12px; height: 12px; flex: none; border: 2px solid var(--ui-accent); border-top-color: transparent; border-radius: 50%; animation: spin .7s linear infinite; }
  button { display: grid; width: 20px; height: 20px; padding: 0; place-items: center; border: 0; border-radius: 3px; background: transparent; color: currentColor; cursor: pointer; }
  button:hover { background: rgb(0 0 0 / .07); } button:focus-visible { outline: 2px solid var(--ui-focus); }
  @keyframes spin { to { transform: rotate(360deg); } }
</style>
