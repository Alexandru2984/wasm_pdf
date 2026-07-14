use yew::prelude::*;

#[function_component(App)]
fn app() -> Html {
    html! { <main>{"PDF Editor"}</main> }
}

fn main() {
    yew::Renderer::<App>::new().render();
}
