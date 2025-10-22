import content from "./index.html?raw";

const template = document.createElement("template");
template.innerHTML = content;

customElements.define(
    "section-container",
    class extends HTMLElement {
        static observedAttributes = ["open"];

        constructor() {
            super();

            const clone = template.content.cloneNode(true);
            const shadowRoot = this.attachShadow({ mode: "open" });
            shadowRoot.appendChild(clone);
        }

        connectedCallback() {
            const header = this.shadowRoot?.querySelector("#header");

            header?.addEventListener("click", () => {
                if (this.getAttribute("open") != null) {
                    this.removeAttribute("open");
                } else {
                    this.setAttribute("open", "");
                }
            });
        }

        attributeChangedCallback(
            name: string,
            _oldValue: string | null,
            newValue: string | null,
        ) {
            if (name === "open") {
                const open = newValue != null;

                const chevron = this.shadowRoot?.querySelector("#chevron");
                const content = this.shadowRoot?.querySelector("#content");

                if (open) {
                    chevron?.classList.add("open");
                    content?.classList.add("open");
                } else {
                    chevron?.classList.remove("open");
                    content?.classList.remove("open");
                }
            }
        }
    },
);
