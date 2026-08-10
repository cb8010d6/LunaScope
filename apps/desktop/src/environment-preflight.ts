type CapabilityWorker = {
  role: string;
  tags: string[];
  task: string;
  prompt: string;
  tools: string[];
  completionCriteria: string[];
};

type CapabilityPlan = {
  objective: string;
  workers: CapabilityWorker[];
};

const rustTerms = /\b(rust|cargo|\.rs)\b|编译|后端|锈语言/iu;
const nodeTerms = /\b(node|npm|typescript|javascript|vite|react|vue|svelte|frontend|html|css)\b|前端|网页|浏览器/iu;
const pythonTerms = /\b(python|pytest|pyproject|pip|\.py)\b|蟒蛇|数据分析/iu;

export function requiredCapabilitiesForPlan(
  plan: CapabilityPlan,
  workspaceManifests: string[],
): string[] {
  const capabilities = new Set<string>();
  const manifests = new Set(workspaceManifests);
  for (const worker of plan.workers) {
    const tools = new Set(worker.tools);
    if (tools.has("check_browser_page")) capabilities.add("browser");
    for (const capability of ["node", "npm", "python", "cargo", "browser"]) {
      if (tools.has(capability)) capabilities.add(capability);
    }
    const canRunVerification = tools.has("process.run");
    const changesCode = tools.has("filesystem.patch") || tools.has("filesystem.write");
    if (!canRunVerification || !changesCode) continue;
    const description = [
      plan.objective,
      worker.role,
      ...worker.tags,
      worker.task,
      worker.prompt,
      ...worker.completionCriteria,
    ].join("\n");
    if (manifests.has("Cargo.toml") && rustTerms.test(description)) {
      capabilities.add("cargo");
    }
    if (manifests.has("package.json") && nodeTerms.test(description)) {
      capabilities.add("node");
      capabilities.add("npm");
    }
    if (
      (manifests.has("pyproject.toml") || manifests.has("requirements.txt")) &&
      pythonTerms.test(description)
    ) {
      capabilities.add("python");
    }
  }
  return [...capabilities].sort();
}
