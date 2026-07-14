import { DocsSearch } from "@/components/docs-search";
import { buildPageMetadata } from "@/lib/page-meta";

export async function generateMetadata({ params }: { params: Promise<{ locale: string }> }) {
  const { locale } = await params;
  const isZh = locale === "zh";
  return buildPageMetadata({
    path: "/docs",
    locale,
    title: isZh ? "文档 · Codewhale" : "Docs · Codewhale",
    description: isZh
      ? "Codewhale 文档：安装、使用指南、配置、提供商、核心概念、工具、MCP、技能、沙箱、运行时 API、排障。"
      : "Codewhale documentation: install, user guide, configuration, providers, core concepts, tools, MCP, skills, sandbox, runtime API, troubleshooting.",
  });
}

export default async function DocsHubPage({ params }: { params: Promise<{ locale: string }> }) {
  const { locale } = await params;
  return <DocsSearch locale={locale} />;
}
