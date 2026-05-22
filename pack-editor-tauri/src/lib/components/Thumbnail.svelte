<script lang="ts">
  // Thumbnail.svelte
  interface FileInfo {
    type: 'image' | 'video' | 'audio';
    width?: number;
    height?: number;
    transparent?: boolean;
    duration?: number;
    audio?: boolean;
  }

  interface Media {
    id: number;
    file_info: FileInfo;
    file_name: string;
    selected: boolean;
    hash: string;
  }

  let { 
    file, 
    port = "", 
    onclick, 
    ondoubleclick 
  } = $props<{
    file: Media;
    port?: string;
    onclick: (e: MouseEvent) => void;
    ondoubleclick: (e: MouseEvent) => void;
  }>();

  // Helper to format duration (e.g. 74.5 -> "1:14")
  function formatDuration(sec?: number): string {
    if (sec === undefined) return "";
    const m = Math.floor(sec / 60);
    const s = Math.floor(sec % 60).toString().padStart(2, "0");
    return `${m}:${s}`;
  }

  // Derive thumbnail source URL or if we should use local mock gradients
  let isMock = $derived(!port || file.hash.startsWith("mock-"));
  let imgSrc = $derived(isMock ? "" : `http://localhost:${port}/thumbnail/${file.id}?hash=${file.hash}`);
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<div 
  class="thumbnail-card"
  class:selected={file.selected}
  onclick={onclick}
  ondblclick={ondoubleclick}
  role="gridcell"
  id={`thumbnail-${file.id}`}
>
  <!-- Thumbnail Image/Icon Area -->
  <div class="media-preview-container">
    {#if file.file_info.type === 'audio'}
      <!-- Audio Icon Fallback with Premium Wave Gradient -->
      <div class="media-icon-wrapper audio-gradient">
        <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" class="type-icon">
          <path stroke-linecap="round" stroke-linejoin="round" d="M9 9l10.5-3m0 0v1.5m0-1.5L9 12m10.5-6v15M9 12v9m10.5-9h-10.5M9 12L3 14.25M9 21H3.75a.75.75 0 01-.75-.75V15L9 21z" />
        </svg>
      </div>
    {:else if isMock}
      <!-- Image/Video Mock Placeholder Gradients -->
      <div class="media-icon-wrapper" class:image-gradient={file.file_info.type === 'image'} class:video-gradient={file.file_info.type === 'video'}>
        {#if file.file_info.type === 'video'}
          <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" class="type-icon">
            <path stroke-linecap="round" stroke-linejoin="round" d="M5.25 5.653c0-.856.917-1.398 1.667-.986l11.54 6.348a1.125 1.125 0 010 1.971l-11.54 6.347a1.125 1.125 0 01-1.667-.985V5.653z" />
          </svg>
        {:else}
          <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" class="type-icon">
            <path stroke-linecap="round" stroke-linejoin="round" d="M2.25 15.75l5.159-5.159a2.25 2.25 0 013.182 0l5.159 5.159m-1.5-1.5l1.409-1.409a2.25 2.25 0 013.182 0l2.909 2.909m-18 3.75h16.5a1.5 1.5 0 001.5-1.5V6a1.5 1.5 0 00-1.5-1.5H3.75A1.5 1.5 0 002.25 6v12a1.5 1.5 0 001.5 1.5zm10.5-11.25h.008v.008h-.008V8.25zm.375 0a.375 0 11-.75 0 .375 0 01.75 0z" />
          </svg>
        {/if}
      </div>
    {:else}
      <!-- Actual Image Rendered lazily -->
      <img 
        class="media-image"
        class:image-selected={file.selected}
        src={imgSrc}
        alt={file.file_name}
        loading="lazy"
      />
    {/if}

    <!-- Selection Checkbox overlay -->
    <div class="checkbox-overlay" class:visible={file.selected}>
      <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor" class="check-icon">
        <path fill-rule="evenodd" d="M2.25 12c0-5.385 4.365-9.75 9.75-9.75s9.75 4.365 9.75 9.75-4.365 9.75-9.75 9.75S2.25 17.385 2.25 12zm13.36-1.814a.75.75 0 10-1.22-.872l-3.236 4.53L9.53 12.22a.75.75 0 00-1.06 1.06l2.25 2.25a.75.75 0 001.14-.094l3.75-5.25z" clip-rule="evenodd" />
      </svg>
    </div>

    <!-- Duration Badge overlay for video -->
    {#if file.file_info.type === 'video' && file.file_info.duration !== undefined}
      <span class="duration-badge">{formatDuration(file.file_info.duration)}</span>
    {/if}
  </div>

  <!-- Filename Text -->
  <p class="file-name-text" title={file.file_name}>
    {file.file_name}
  </p>
</div>

<style>
  .thumbnail-card {
    width: 150px;
    height: 200px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: flex-start;
    padding: 10px;
    border-radius: 12px;
    background-color: var(--card-bg, rgba(255, 255, 255, 0.03));
    border: 1px solid var(--card-border, rgba(255, 255, 255, 0.06));
    cursor: pointer;
    user-select: none;
    transition: all 0.25s cubic-bezier(0.4, 0, 0.2, 1);
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.15);
  }

  .thumbnail-card:hover {
    transform: translateY(-4px);
    background-color: var(--card-hover-bg, rgba(255, 255, 255, 0.08));
    border-color: var(--card-hover-border, rgba(255, 255, 255, 0.12));
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.25);
  }

  .thumbnail-card.selected {
    background-color: rgba(14, 165, 233, 0.15);
    border-color: #0ea5e9;
    box-shadow: 0 0 0 1px #0ea5e9, 0 8px 24px rgba(14, 165, 233, 0.15);
  }

  .media-preview-container {
    position: relative;
    width: 130px;
    height: 130px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 8px;
    overflow: hidden;
    background-color: rgba(0, 0, 0, 0.2);
  }

  .media-image {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
    transition: transform 0.3s ease;
  }

  .thumbnail-card:hover .media-image {
    transform: scale(1.05);
  }

  .media-image.image-selected {
    filter: brightness(0.95);
  }

  .media-icon-wrapper {
    width: 100%;
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: transform 0.3s ease;
  }

  .thumbnail-card:hover .media-icon-wrapper {
    transform: scale(1.06);
  }

  .type-icon {
    width: 48px;
    height: 48px;
    color: rgba(255, 255, 255, 0.7);
    filter: drop-shadow(0 2px 8px rgba(0, 0, 0, 0.3));
  }

  /* Premium mock gradients */
  .audio-gradient {
    background: linear-gradient(135deg, #0f172a 0%, #1e40af 100%);
  }
  
  .image-gradient {
    background: linear-gradient(135deg, #450a0a 0%, #991b1b 100%);
  }

  .video-gradient {
    background: linear-gradient(135deg, #3b0764 0%, #6b21a8 100%);
  }

  /* Overlay states */
  .checkbox-overlay {
    position: absolute;
    top: 6px;
    left: 6px;
    opacity: 0;
    transform: scale(0.8);
    transition: all 0.2s ease;
  }

  .checkbox-overlay.visible,
  .thumbnail-card:hover .checkbox-overlay {
    opacity: 1;
    transform: scale(1);
  }

  .check-icon {
    width: 22px;
    height: 22px;
    color: #0ea5e9;
    background-color: rgba(15, 23, 42, 0.9);
    border-radius: 50%;
    box-shadow: 0 2px 6px rgba(0, 0, 0, 0.3);
  }

  .duration-badge {
    position: absolute;
    bottom: 6px;
    right: 6px;
    background-color: rgba(15, 23, 42, 0.85);
    backdrop-filter: blur(4px);
    color: #f8fafc;
    font-size: 10px;
    font-weight: 600;
    padding: 2px 6px;
    border-radius: 4px;
    letter-spacing: 0.5px;
    box-shadow: 0 2px 4px rgba(0, 0, 0, 0.3);
    border: 1px solid rgba(255, 255, 255, 0.05);
  }

  .file-name-text {
    width: 100%;
    margin-top: 10px;
    font-size: 11px;
    line-height: 1.4;
    font-weight: 500;
    color: var(--text-muted, #94a3b8);
    text-align: center;
    overflow: hidden;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    text-overflow: ellipsis;
    word-break: break-all;
    transition: color 0.2s;
  }

  .thumbnail-card:hover .file-name-text {
    color: var(--text-active, #f1f5f9);
  }

  .thumbnail-card.selected .file-name-text {
    color: #e0f2fe;
    font-weight: 600;
  }
</style>
