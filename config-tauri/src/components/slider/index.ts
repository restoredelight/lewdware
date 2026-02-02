import content from "./index.html?raw";

const template = document.createElement("template");
template.innerHTML = content;

export class HTMLSliderElement extends HTMLElement {
    static observedAttributes = ["value"];

    private default: number = 0;

    constructor() {
        super();

        const clone = template.content.cloneNode(true);
        const shadowRoot = this.attachShadow({ mode: "open" });
        shadowRoot.appendChild(clone);
    }

    connectedCallback() {
        const slider: HTMLInputElement | null | undefined =
            this.shadowRoot?.querySelector("#slider");
        const input: HTMLInputElement | null | undefined =
            this.shadowRoot?.querySelector("#input");
        const container = this.shadowRoot?.querySelector("#container");

        if (this.getAttribute("horizontal") != null) {
            container?.classList.add("horizontal");
        }

        const min = parseFloat(this.getAttribute("min") ?? "");
        const max = parseFloat(this.getAttribute("max") ?? "");

        let clampPositive = false;
        let clampBottom = false;
        let clampTop = false;
        const clamp = this.getAttribute("clamp");

        if (clamp === "lower") {
            clampBottom = true;
        } else if (clamp === "upper") {
            clampTop = true;
        } else if (clamp === "positive") {
            clampPositive = true;
        } else if (clamp !== null) {
            clampBottom = true;
            clampTop = true;
        }

        const step = parseFloat(this.getAttribute("step") ?? "1");

        const parts = step.toString().split(".");
        const decimals = parts.length > 1 ? parts[1].length : 0;

        if (slider) {
            slider.min = min.toString();
            slider.max = max.toString();
            slider.step = step.toString();
        }

        if (input) {
            input.step = step.toString();
        }

        slider?.addEventListener("input", () => {
            if (input) {
                input.value = parseFloat(slider.value).toFixed(decimals);
            }
        });

        input?.addEventListener("input", () => {
            if (slider) {
                slider.value = input.value;
            }
        });

        slider?.addEventListener("change", () => {
            this.dispatchEvent(
                new CustomEvent("change", {
                    detail: parseFloat(slider.value),
                }),
            );
        });

        input?.addEventListener("change", () => {
            const value = parseFloat(input.value);

            let finalValue = value;

            if (isNaN(value)) {
                finalValue = this.default;
            }

            if (clampPositive) {
                finalValue = Math.max(0, finalValue);
            }

            if (clampBottom) {
                finalValue = Math.max(min, finalValue);
            }

            if (clampTop) {
                finalValue = Math.min(max, finalValue);
            }

            const roundedValue = finalValue.toFixed(decimals);

            if (parseFloat(roundedValue) != value) {
                input.value = roundedValue;
                input.dispatchEvent(new Event("input"));
            } else if (roundedValue != input.value) {
                input.value = roundedValue;
            }

            this.dispatchEvent(
                new CustomEvent("change", {
                    detail: parseFloat(roundedValue),
                }),
            );
        });

        input?.addEventListener("focus", () => {
            setTimeout(() => input.select(), 0);
        });
    }

    get value(): number {
        const slider: HTMLInputElement | null | undefined =
            this.shadowRoot?.querySelector("#slider");

        return parseFloat(slider?.value ?? "0");
    }

    set value(value: number) {
        this.setAttribute("value", value.toString());
    }

    attributeChangedCallback(
        name: string,
        _oldValue: string | null,
        newValue: string | null,
    ) {
        if (name === "value") {
            const slider: HTMLInputElement | null | undefined =
                this.shadowRoot?.querySelector("#slider");
            const input: HTMLInputElement | null | undefined =
                this.shadowRoot?.querySelector("#input");

            const step = parseFloat(this.getAttribute("step") ?? "1");
            const parts = step.toString().split(".");
            const decimals = parts.length > 1 ? parts[1].length : 0;

            const value = parseFloat(newValue ?? "0");

            if (slider) {
                slider.value = value.toFixed(decimals);
            }

            if (input) {
                input.value = value.toFixed(decimals);
            }

            this.default = value;
        }
    }
}

customElements.define("slider-input", HTMLSliderElement);
