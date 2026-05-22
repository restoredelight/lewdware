import { HTMLSliderElement } from "./components/slider";
import { Config } from "./state";

export function setupExtraSection(config: Config) {
    const linksCheckbox: HTMLInputElement | null =
        document.querySelector("#open-links");
    const linkContainer = document.querySelector("#link-frequency-container");

    linksCheckbox?.addEventListener("change", () => {
        config.open_links = linksCheckbox.checked;

        if (linksCheckbox.checked && linkContainer) {
            linkContainer.classList.remove("hidden");
        } else if (linkContainer) {
            linkContainer.classList.add("hidden");
        }
    });

    if (config.open_links) {
        if (linksCheckbox && linkContainer) {
            linksCheckbox.checked = true;
            linkContainer.classList.remove("hidden");
        }
    }

    const linkFrequency: HTMLSliderElement | null =
        document.querySelector("#link-frequency");

    if (linkFrequency) {
        linkFrequency.value = config.link_frequency;
    }

    linkFrequency?.addEventListener("change", () => {
        config.link_frequency = linkFrequency.value;
    });

    const notificationsCheckbox: HTMLInputElement | null =
        document.querySelector("#notifications");

    const notificationContainer = document.querySelector(
        "#notification-frequency-container",
    );

    notificationsCheckbox?.addEventListener("change", () => {
        config.notifications = notificationsCheckbox.checked;

        if (notificationsCheckbox.checked && notificationContainer) {
            notificationContainer.classList.remove("hidden");
        } else if (notificationContainer) {
            notificationContainer.classList.add("hidden");
        }
    });

    if (config.notifications) {
        if (notificationsCheckbox && notificationContainer) {
            notificationsCheckbox.checked = true;
            notificationContainer.classList.remove("hidden");
        }
    }

    const notificationFrequency: HTMLSliderElement | null =
        document.querySelector("#notification-frequency");

    if (notificationFrequency) {
        notificationFrequency.value = config.notification_frequency;
    }

    notificationFrequency?.addEventListener("change", () => {
        config.notification_frequency = notificationFrequency.value;
    });

    const promptsCheckbox: HTMLInputElement | null = document.querySelector(
        "#prompts",
    );

    const promptContainer = document.querySelector(
        "#prompt-frequency-container",
    );

    promptsCheckbox?.addEventListener("change", () => {
        config.prompts = promptsCheckbox.checked;

        if (promptsCheckbox.checked && promptContainer) {
            promptContainer.classList.remove("hidden");
        } else if (promptContainer) {
            promptContainer.classList.add("hidden");
        }
    });

    if (config.prompts) {
        if (promptsCheckbox && promptContainer) {
            promptsCheckbox.checked = true;
            promptContainer.classList.remove("hidden");
        }
    }

    const promptFrequency: HTMLSliderElement | null = document.querySelector("#prompt-frequency");

    if (promptFrequency) {
        promptFrequency.value = config.prompt_frequency;
    }

    promptFrequency?.addEventListener("change", () => {
        config.prompt_frequency = promptFrequency.value;
    });
}
