import { HTMLSliderElement } from "./components/slider";
import { Config } from "./state";

export function setupPopupsSection(config: Config) {
    const movingWindowCheckbox: HTMLInputElement | null =
        document.querySelector("#moving-windows");
    const movingWindowsContainer = document.querySelector(
        "#move-chance-container",
    );

    movingWindowCheckbox?.addEventListener("change", () => {
        config.moving_windows = movingWindowCheckbox.checked

        if (movingWindowCheckbox.checked && movingWindowsContainer) {
            movingWindowsContainer.classList.remove("hidden");
        } else if (movingWindowsContainer) {
            movingWindowsContainer.classList.add("hidden");
        }
    });

    if (movingWindowCheckbox) {
        movingWindowCheckbox.checked = config.moving_windows;
    }

    if (config.moving_windows) {
        if (movingWindowCheckbox && movingWindowsContainer) {
            movingWindowCheckbox.checked = true;
            movingWindowsContainer.classList.remove("hidden");
        }
    }

    const moveChance: HTMLSliderElement | null = document.querySelector("#move-chance");

    moveChance?.addEventListener("change", () => {
        config.moving_window_chance = moveChance.value;
    });

    if (moveChance) {
        moveChance.value = config.moving_window_chance;
    }

    const closeButton: HTMLInputElement | null = document.querySelector(
        "#close-button",
    );
    const closeTip = document.querySelector("#close-tip");

    closeButton?.addEventListener("change", () => {
        config.close_button = closeButton.checked;

        if (closeButton.checked && closeTip) {
            closeTip.classList.add("hidden");
        } else if (closeTip) {
            closeTip.classList.remove("hidden");
        }
    });

    if (config.close_button) {
        if (closeButton) {
            closeButton.checked = true;
        }
    } else if (closeTip) {
        closeTip.classList.remove("hidden");
    }
}
