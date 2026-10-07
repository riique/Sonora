import ReactMarkdown from "react-markdown";
import { MessageCircleMore } from "lucide-react";

/**
 * Renders the persisted pronunciation report without assuming a fixed model
 * response schema. The evaluator returns Markdown and older history entries
 * may use different headings, so the renderer keeps every section while
 * presenting it with the same quiet list grammar as the rest of the product.
 */
export function PronunciationEvaluation({ markdown }: { markdown: string }) {
  return (
    <section className="border-t border-line pt-5" aria-label="Avaliação de pronúncia">
      <header className="flex items-center gap-3">
        <span className="flex h-8 w-8 items-center justify-center rounded-full bg-fill text-soft">
          <MessageCircleMore className="h-4 w-4" aria-hidden />
        </span>
        <div>
          <h3 className="text-[13px] font-medium text-ink">Avaliação de pronúncia</h3>
          <p className="mt-0.5 text-[11px] text-muted">Feedback salvo para este áudio</p>
        </div>
      </header>
      <div className="pronunciation-report max-w-[68ch] pt-4 text-[13px] leading-6 text-strong">
        <ReactMarkdown
          components={{
            h1: ({ children }) => <h4 className="mb-3 mt-6 first:mt-0 text-[16px] font-semibold tracking-[-0.01em] text-ink">{children}</h4>,
            h2: ({ children }) => <h4 className="mb-2 mt-6 first:mt-0 border-b border-line pb-2 text-[14px] font-semibold text-ink">{children}</h4>,
            h3: ({ children }) => <h5 className="mb-2 mt-5 text-[13px] font-medium text-ink">{children}</h5>,
            p: ({ children }) => <p className="my-2 text-[13px] leading-6 text-strong">{children}</p>,
            ul: ({ children }) => <ul className="my-3 space-y-1.5 pl-5 marker:text-faint">{children}</ul>,
            ol: ({ children }) => <ol className="my-3 list-decimal space-y-1.5 pl-5 marker:text-faint">{children}</ol>,
            li: ({ children }) => <li className="pl-1 text-[13px] leading-6 text-strong">{children}</li>,
            strong: ({ children }) => <strong className="font-semibold text-ink">{children}</strong>,
            em: ({ children }) => <em className="text-muted">{children}</em>,
            blockquote: ({ children }) => <blockquote className="my-4 border-l-2 border-line-strong pl-4 text-muted">{children}</blockquote>,
            code: ({ children }) => <code className="rounded-[5px] bg-fill px-1.5 py-0.5 font-mono text-[11px] text-ink">{children}</code>,
            pre: ({ children }) => <pre className="my-4 max-h-72 overflow-auto whitespace-pre-wrap rounded-[8px] bg-fill p-4 font-mono text-[11px] leading-5 text-strong">{children}</pre>,
            hr: () => <hr className="my-5 border-line" />,
            table: ({ children }) => <div className="my-4 overflow-auto"><table className="w-full border-collapse text-left text-[12px]">{children}</table></div>,
            th: ({ children }) => <th className="border-b border-line px-3 py-2 font-medium text-ink">{children}</th>,
            td: ({ children }) => <td className="border-b border-line px-3 py-2 text-muted">{children}</td>,
          }}
        >
          {markdown}
        </ReactMarkdown>
      </div>
    </section>
  );
}
