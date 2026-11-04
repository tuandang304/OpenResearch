// arXiv and Semantic Scholar discovery sources, owned by the tuandang304 fork.
// orxCommand.ts and LitSourceLogo.tsx spread these into the upstream source
// maps; see docs/fork-maintenance.md.
import arxivSvg from "../../assets/lit-sources/arxiv.svg?raw";
import semanticScholarSvg from "../../assets/lit-sources/semanticscholar.svg?raw";

export type ForkLitSource = "arxiv" | "semanticscholar";

export const FORK_LIT_SOURCES: ForkLitSource[] = ["arxiv", "semanticscholar"];

export const FORK_LIT_SOURCE_NAME: Record<ForkLitSource, string> = {
  arxiv: "arXiv",
  semanticscholar: "Semantic Scholar",
};

// Stable HTML objects for LitSourceLogo, which keeps a pressed SVG target
// through a re-render by reusing the same object.
export const FORK_LIT_SOURCE_HTML: Record<ForkLitSource, { __html: string }> = {
  arxiv: { __html: arxivSvg },
  semanticscholar: { __html: semanticScholarSvg },
};
