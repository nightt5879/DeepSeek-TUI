"use client";

import { useState, useMemo, useRef, useCallback, useEffect } from "react";
import Link from "next/link";
import { DOC_TOPICS, REPO_DOCS_BASE, type DocTopic } from "@/lib/docs-map";
import { docTopicHaystack } from "@/lib/search-utils";

/* ------------------------------------------------------------------ */
/*  Locale-aware strings                                              */
/* ------------------------------------------------------------------ */

const CATEGORY_LABELS: Record<string, { en: string; zh: string }> = {
  "getting-started": { en: "Getting started", zh: "入门" },
  "core-concepts": { en: "Core concepts", zh: "核心概念" },
  reference: { en: "Reference", zh: "参考" },
  extending: { en: "Extending", zh: "扩展" },
  operations: { en: "Operations & community", zh: "运维与社区" },
};

/* ------------------------------------------------------------------ */
/*  Link / source helpers (mirrored from the original page.tsx)       */
/* ------------------------------------------------------------------ */

function topicHref(topic: DocTopic, locale: string): string {
  if (topic.hasPage) {
    return `/${locale}/docs/${topic.slug}`;
  }
  const src = Array.isArray(topic.repoSource) ? topic.repoSource[0] : topic.repoSource;
  return `${REPO_DOCS_BASE}/${src}`;
}

function topicSources(topic: DocTopic): string[] {
  return Array.isArray(topic.repoSource) ? topic.repoSource : [topic.repoSource];
}

/**
 * Build a single lowercase haystack string for fuzzy matching.
 * Delegates to the shared search-utils for testability.
 * Includes both EN and ZH text so a user can search in either language
 * regardless of the active locale.
 */
const topicHaystack = docTopicHaystack;

/* ------------------------------------------------------------------ */
/*  Highlight helper                                                   */
/* ------------------------------------------------------------------ */

function highlight(text: string, query: string): React.ReactNode {
  const q = query.trim().toLowerCase();
  if (!q) return text;
  const lower = text.toLowerCase();
  const idx = lower.indexOf(q);
  if (idx === -1) return text;
  return (
    <>
      {text.slice(0, idx)}
      <mark className="search-highlight">{text.slice(idx, idx + q.length)}</mark>
      {text.slice(idx + q.length)}
    </>
  );
}

/* ------------------------------------------------------------------ */
/*  Topic card                                                         */
/* ------------------------------------------------------------------ */

function TopicCard({
  topic,
  locale,
  query,
}: {
  topic: DocTopic;
  locale: string;
  query: string;
}) {
  const isZh = locale === "zh";
  const href = topicHref(topic, locale);
  const sources = topicSources(topic);
  const isExternal = !topic.hasPage;

  return (
    <Link
      href={href}
      target={isExternal ? "_blank" : undefined}
      className="hairline-t hairline-b hairline-l hairline-r p-4 hover:bg-paper-deep transition-colors group block"
    >
      <div className="flex items-center gap-2 mb-1.5">
        <span className="font-mono text-[0.62rem] uppercase tracking-widest text-ink-mute">
          {highlight(isZh ? topic.label.zh : topic.label.en, query)}
        </span>
        {isExternal && (
          <span className="font-mono text-[0.6rem] text-ink-mute">↗</span>
        )}
      </div>
      <p className="text-sm text-ink-soft leading-relaxed">
        {highlight(isZh ? topic.description.zh : topic.description.en, query)}
      </p>
      <div className="mt-2 font-mono text-[0.62rem] text-ink-mute truncate">
        {sources.map((s, i) => (
          <span key={s}>
            {i > 0 && ", "}
            {highlight(s, query)}
          </span>
        ))}
      </div>
    </Link>
  );
}

/* ------------------------------------------------------------------ */
/*  Main component                                                     */
/* ------------------------------------------------------------------ */

export function DocsSearch({ locale }: { locale: string }) {
  const isZh = locale === "zh";
  const [query, setQuery] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  // Precompute haystacks once.
  const haystacks = useMemo(() => DOC_TOPICS.map(topicHaystack), []);

  // Filter topics by query.
  const filteredTopics = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return DOC_TOPICS;
    return DOC_TOPICS.filter((_, i) => haystacks[i].includes(q));
  }, [query, haystacks]);

  // Group filtered topics by category (preserve DOC_TOPICS order).
  const grouped = useMemo(() => {
    const map = new Map<string, DocTopic[]>();
    for (const t of filteredTopics) {
      const group = map.get(t.category) ?? [];
      group.push(t);
      map.set(t.category, group);
    }
    return map;
  }, [filteredTopics]);

  // Keyboard shortcut: focus search on "/".
  const handleKeyDown = useCallback((e: KeyboardEvent) => {
    if (e.key === "/" && document.activeElement?.tagName !== "INPUT") {
      e.preventDefault();
      inputRef.current?.focus();
    }
  }, []);

  useEffect(() => {
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [handleKeyDown]);

  const total = DOC_TOPICS.length;
  const matched = filteredTopics.length;
  const hasQuery = query.trim().length > 0;

  return (
    <div>
      {/* Search bar */}
      <div className="mb-8">
        <div className="relative">
          <input
            ref={inputRef}
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={
              isZh
                ? "搜索文档…（按 / 快速聚焦）"
                : "Search docs… (press / to focus)"
            }
            className="search-input w-full"
            aria-label={isZh ? "搜索文档" : "Search documentation"}
          />
          {hasQuery && (
            <button
              onClick={() => setQuery("")}
              className="absolute right-3 top-1/2 -translate-y-1/2 font-mono text-sm text-ink-mute hover:text-indigo transition-colors"
              aria-label={isZh ? "清除" : "Clear"}
            >
              ✕
            </button>
          )}
        </div>
        {hasQuery && (
          <div className="mt-2 font-mono text-[0.7rem] text-ink-mute">
            {matched > 0
              ? isZh
                ? `${matched} / ${total} 篇文档匹配 "${query.trim()}"`
                : `${matched} of ${total} docs match "${query.trim()}"`
              : isZh
                ? `未找到匹配 "${query.trim()}" 的文档`
                : `No docs match "${query.trim()}"`}
          </div>
        )}
      </div>

      {/* Results */}
      {matched > 0 ? (
        <div className="space-y-12">
          {[...grouped.entries()].map(([cat, topics]) => (
            <section key={cat} id={cat}>
              <h2 className="font-display text-2xl mb-1">
                {isZh ? CATEGORY_LABELS[cat]?.zh ?? cat : CATEGORY_LABELS[cat]?.en ?? cat}
              </h2>
              <div className="grid sm:grid-cols-2 gap-4 mt-4">
                {topics.map((t) => (
                  <TopicCard key={t.id} topic={t} locale={locale} query={query} />
                ))}
              </div>
            </section>
          ))}
        </div>
      ) : (
        <div className="text-center py-16">
          <p className="font-display text-lg text-ink-mute mb-2">
            {isZh ? "未找到结果" : "No results found"}
          </p>
          <p className="text-sm text-ink-mute">
            {isZh
              ? "尝试使用不同的关键字，或浏览 GitHub 上的完整文档。"
              : "Try a different keyword, or browse the full docs on GitHub."}
          </p>
          <Link
            href="https://github.com/Hmbown/CodeWhale/tree/main/docs"
            target="_blank"
            className="inline-flex items-center gap-2 mt-4 px-4 py-2 hairline-t hairline-b hairline-l hairline-r font-mono text-[0.7rem] uppercase tracking-wider hover:bg-paper-deep transition-colors"
          >
            {isZh ? "GitHub 文档目录 ↗" : "GitHub docs directory ↗"}
          </Link>
        </div>
      )}

      {/* Footer note (only when not searching) */}
      {!hasQuery && (
        <section className="hairline-t pt-8 mt-12">
          <p className="text-sm text-ink-mute leading-relaxed max-w-2xl">
            {isZh
              ? "§ 标记的条目在 Codewhale 网站上有独立页面；↗ 标记的条目链接到 GitHub 仓库中的源文档。所有内容来源于 docs/ 目录下的 40+ 篇 Markdown 文档，通过 docs-map.ts 注册表维护。"
              : "Entries marked § have dedicated pages on codewhale.net; entries marked ↗ link to source documents in the GitHub repository. All content is sourced from 40+ Markdown documents in the docs/ directory, maintained through the docs-map.ts registry."}
          </p>
        </section>
      )}
    </div>
  );
}
