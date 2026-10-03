import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import Markdown from "react-markdown";

let notes = "";
for await (const chunk of process.stdin) notes += chunk;
process.stdout.write(
  renderToStaticMarkup(
    createElement(Markdown, {
      children: notes,
      components: { img: () => null },
    }),
  ),
);
