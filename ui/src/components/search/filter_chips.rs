use dioxus::prelude::*;

#[derive(Props, PartialEq, Clone)]
pub struct Props {
    pub label: String,
    pub values: Vec<String>,
    pub selected: Vec<String>,
    #[props(into)]
    pub on_toggle: EventHandler<String>,
}

fn chip(label: String, active: bool, on_toggle: EventHandler<String>) -> Element {
    let clicked = label.clone();
    let text = label.clone();
    rsx! {
      button {
        key: "{label}",
        class: if active {
          "px-2 py-0.5 rounded border text-[11px] font-mono bg-beet-accent/20 border-beet-accent/50 text-beet-accent cursor-pointer"
        } else {
          "px-2 py-0.5 rounded border text-[11px] font-mono bg-white/5 border-white/10 text-gray-400 hover:text-white hover:border-white/20 cursor-pointer"
        },
        onclick: move |_| on_toggle.call(clicked.clone()),
        "{text}"
      }
    }
}

#[component]
pub fn FilterChips(props: Props) -> Element {
    let chips: Vec<(String, bool)> = props
        .values
        .iter()
        .map(|value| (value.clone(), props.selected.contains(value)))
        .collect();

    rsx! {
      div { class: "flex items-start gap-3",
        span { class: "w-24 shrink-0 inline-block pt-1 text-[10px] font-mono uppercase tracking-widest text-gray-500",
          "{props.label}"
        }
        div { class: "flex flex-1 min-w-0 flex-wrap items-center gap-1.5",
          for (value, is_active) in chips {
            {chip(value, is_active, props.on_toggle)}
          }
        }
      }
    }
}
