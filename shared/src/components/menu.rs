use dioxus::prelude::*;
use dioxus_heroicons::{Icon, solid::Shape};

pub trait MenuItem: std::fmt::Display + Sized + Copy + PartialEq + 'static {
    const VARIANTS: &'static [Self];

    fn icon(&self) -> Shape;
}

#[component]
pub fn Menu<T: MenuItem>(initially_open: bool, selected: Signal<T>) -> Element {
    let mut open = use_signal(|| initially_open);

    let options = 
        T::VARIANTS.iter().map(|&option| {
            rsx! {
                button {
                    class: "flex items-center p-1 rounded-sm w-full nowrap transition-[width]",
                    class: if selected() == option { "bg-sky-400 text-white" } else { "hover:bg-sky-200" },
                    onclick: move |_| {
                        selected.set(option);
                    },
                    Icon {
                        class: "mx-1",
                        icon: option.icon(),
                        size: 20,
                    }
                    "{option}"
                }
            }
        });

    rsx! {
        div {
            class: "p-[5px] transition-[width] border-r border-gray-300 bg-gray-50 overflow-hidden",
            width: if open() {"16rem"} else {"40px"},
            div {
                class: "w-full h-[35px] border-b border-gray-300",
                button {
                    class: "float-right rounded-sm hover:text-gray-800 hover:bg-gray-200 size-[30px] flex justify-center items-center",
                    onclick: move |_| {
                        open.toggle();
                    },
                    Icon {
                        icon: Shape::Bars3,
                        size: 25,
                    }
                }
            }
            if open() {
                div {
                    class: "flex flex-col gap-1 pt-2",
                    {options}
                }
            }
        }
    }
}
