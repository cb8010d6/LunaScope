type EnhancedSelect = HTMLSelectElement & { dataset: DOMStringMap & { lunaEnhanced?: string } };

let openShell: HTMLElement | null = null;
let installed = false;
let selectSequence = 0;
let dialogQueue: Promise<void> = Promise.resolve();

export type AppDialogOptions = {
  title: string;
  message: string;
  confirmLabel: string;
  cancelLabel?: string;
  danger?: boolean;
};

function selectedLabel(select: HTMLSelectElement): string {
  return select.selectedOptions[0]?.textContent?.trim() || select.value;
}

function closeSelect(shell: HTMLElement, restoreFocus = false): void {
  const button = shell.querySelector<HTMLButtonElement>(".luna-select-trigger");
  const listbox = shell.querySelector<HTMLElement>(".luna-select-listbox");
  shell.classList.remove("is-open");
  button?.setAttribute("aria-expanded", "false");
  if (listbox) listbox.hidden = true;
  if (openShell === shell) openShell = null;
  if (restoreFocus) button?.focus();
}

function syncSelect(shell: HTMLElement, select: HTMLSelectElement): void {
  const button = shell.querySelector<HTMLButtonElement>(".luna-select-trigger");
  if (!button) return;
  button.textContent = selectedLabel(select);
  button.disabled = select.disabled;
  button.setAttribute("aria-disabled", String(select.disabled));
}

function renderOptions(shell: HTMLElement, select: HTMLSelectElement): void {
  const listbox = shell.querySelector<HTMLElement>(".luna-select-listbox");
  if (!listbox) return;
  listbox.replaceChildren(
    ...Array.from(select.options).map((option) => {
      const item = document.createElement("button");
      item.type = "button";
      item.className = "luna-select-option";
      item.setAttribute("role", "option");
      item.dataset.value = option.value;
      item.textContent = option.textContent;
      item.disabled = option.disabled;
      item.setAttribute("aria-selected", String(option.selected));
      if (option.selected) item.classList.add("is-selected");
      item.addEventListener("click", () => {
        if (option.disabled) return;
        select.value = option.value;
        select.dispatchEvent(new Event("input", { bubbles: true }));
        select.dispatchEvent(new Event("change", { bubbles: true }));
        syncSelect(shell, select);
        closeSelect(shell, true);
      });
      return item;
    }),
  );
}

function moveSelection(shell: HTMLElement, direction: number): void {
  const options = Array.from(
    shell.querySelectorAll<HTMLButtonElement>(".luna-select-option:not(:disabled)"),
  );
  if (!options.length) return;
  const focused = document.activeElement;
  const current = options.findIndex((option) => option === focused);
  options[(current + direction + options.length) % options.length]?.focus();
}

function openSelect(shell: HTMLElement, select: HTMLSelectElement): void {
  if (select.disabled) return;
  if (openShell && openShell !== shell) closeSelect(openShell);
  renderOptions(shell, select);
  const button = shell.querySelector<HTMLButtonElement>(".luna-select-trigger");
  const listbox = shell.querySelector<HTMLElement>(".luna-select-listbox");
  if (!button || !listbox) return;
  shell.classList.add("is-open");
  button.setAttribute("aria-expanded", "true");
  listbox.hidden = false;
  openShell = shell;
  queueMicrotask(() =>
    (listbox.querySelector<HTMLElement>("[aria-selected='true']") ??
      listbox.querySelector<HTMLElement>(".luna-select-option:not(:disabled)"))?.focus(),
  );
}

function enhanceSelect(select: EnhancedSelect): void {
  if (select.dataset.lunaEnhanced === "true" || select.multiple || select.size > 1) return;
  select.dataset.lunaEnhanced = "true";
  const shell = document.createElement("span");
  shell.className = "luna-select";
  const button = document.createElement("button");
  button.type = "button";
  button.className = "luna-select-trigger";
  button.setAttribute("role", "combobox");
  button.setAttribute("aria-haspopup", "listbox");
  button.setAttribute("aria-expanded", "false");
  const label = select.labels?.[0]?.textContent?.trim() || select.getAttribute("aria-label");
  if (label) button.setAttribute("aria-label", label);
  const listbox = document.createElement("span");
  listbox.className = "luna-select-listbox";
  listbox.setAttribute("role", "listbox");
  listbox.id = `luna-select-listbox-${++selectSequence}`;
  listbox.hidden = true;
  button.setAttribute("aria-controls", listbox.id);

  select.before(shell);
  shell.append(select, button, listbox);
  select.classList.add("luna-select-source");
  select.tabIndex = -1;
  select.setAttribute("aria-hidden", "true");
  syncSelect(shell, select);

  button.addEventListener("click", () => {
    if (shell.classList.contains("is-open")) closeSelect(shell);
    else openSelect(shell, select);
  });
  button.addEventListener("keydown", (event) => {
    if (["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
      event.preventDefault();
      openSelect(shell, select);
      const options = Array.from(
        shell.querySelectorAll<HTMLButtonElement>(".luna-select-option:not(:disabled)"),
      );
      if (event.key === "Home") options[0]?.focus();
      else if (event.key === "End") options.at(-1)?.focus();
      else moveSelection(shell, event.key === "ArrowDown" ? 1 : -1);
    }
  });
  listbox.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      event.preventDefault();
      closeSelect(shell, true);
    } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      moveSelection(shell, event.key === "ArrowDown" ? 1 : -1);
    } else if (event.key === "Home" || event.key === "End") {
      event.preventDefault();
      const options = Array.from(
        listbox.querySelectorAll<HTMLButtonElement>(".luna-select-option:not(:disabled)"),
      );
      (event.key === "Home" ? options[0] : options.at(-1))?.focus();
    }
  });
  select.addEventListener("change", () => syncSelect(shell, select));
}

function enhanceWithin(root: ParentNode): void {
  if (root instanceof HTMLSelectElement) {
    enhanceSelect(root as EnhancedSelect);
    if (root.parentElement) syncSelect(root.parentElement, root);
  }
  root.querySelectorAll?.("select").forEach((select) =>
    {
      enhanceSelect(select as EnhancedSelect);
      if (select.parentElement) syncSelect(select.parentElement, select);
    },
  );
}

export function refreshCustomControls(root: ParentNode = document): void {
  enhanceWithin(root);
}

export function installCustomControls(): void {
  if (installed) return;
  installed = true;
  enhanceWithin(document);
  document.documentElement.dataset.customControlsReady = "true";
  new MutationObserver((records) => {
    for (const record of records) {
      for (const node of record.addedNodes) {
        if (node instanceof Element) enhanceWithin(node);
      }
      const select = record.target instanceof Element
        ? record.target.closest<HTMLSelectElement>("select[data-luna-enhanced='true']")
        : null;
      if (select?.parentElement) syncSelect(select.parentElement, select);
    }
  }).observe(document.body, { childList: true, subtree: true });
  document.addEventListener("pointerdown", (event) => {
    if (openShell && event.target instanceof Node && !openShell.contains(event.target)) {
      closeSelect(openShell);
    }
  }, true);
  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && openShell) closeSelect(openShell, true);
  });
  document.addEventListener("input", (event) => {
    if (!(event.target instanceof HTMLSelectElement)) return;
    const shell = event.target.parentElement;
    if (shell?.classList.contains("luna-select")) syncSelect(shell, event.target);
  });
}

function ensureDecisionDialog(): HTMLDialogElement {
  let dialog = document.querySelector<HTMLDialogElement>("#lunaDecisionDialog");
  if (dialog) return dialog;
  dialog = document.createElement("dialog");
  dialog.id = "lunaDecisionDialog";
  dialog.className = "luna-decision-dialog";
  dialog.setAttribute("aria-labelledby", "lunaDecisionDialogTitle");
  dialog.setAttribute("aria-describedby", "lunaDecisionDialogMessage");
  dialog.innerHTML = `
    <div class="dialog decision-dialog" role="document">
      <div class="dialog-head">
        <strong id="lunaDecisionDialogTitle"></strong>
        <button class="icon-btn" type="button" data-luna-dialog-action="cancel" aria-label="Close">×</button>
      </div>
      <div class="dialog-body">
        <p class="decision-dialog-message" id="lunaDecisionDialogMessage"></p>
      </div>
      <div class="dialog-foot">
        <span></span>
        <div class="row">
          <button class="ghost-btn" type="button" data-luna-dialog-action="cancel"></button>
          <button class="primary-btn" type="button" data-luna-dialog-action="confirm"></button>
        </div>
      </div>
    </div>`;
  document.body.append(dialog);
  return dialog;
}

function queueDialog<T>(present: () => Promise<T>): Promise<T> {
  const task = dialogQueue.then(present, present);
  dialogQueue = task.then(() => undefined, () => undefined);
  return task;
}

function presentDialog(options: AppDialogOptions, cancellable: boolean): Promise<boolean> {
  return queueDialog(() => new Promise<boolean>((resolve) => {
    const dialog = ensureDecisionDialog();
    const title = dialog.querySelector<HTMLElement>("#lunaDecisionDialogTitle");
    const message = dialog.querySelector<HTMLElement>("#lunaDecisionDialogMessage");
    const closeButton = dialog.querySelector<HTMLButtonElement>(
      '[data-luna-dialog-action="cancel"].icon-btn',
    );
    const cancelButton = dialog.querySelector<HTMLButtonElement>(
      '.dialog-foot [data-luna-dialog-action="cancel"]',
    );
    const confirmButton = dialog.querySelector<HTMLButtonElement>(
      '[data-luna-dialog-action="confirm"]',
    );
    if (!title || !message || !closeButton || !cancelButton || !confirmButton) {
      resolve(false);
      return;
    }

    const previousFocus = document.activeElement instanceof HTMLElement
      ? document.activeElement
      : null;
    title.textContent = options.title;
    message.textContent = options.message;
    confirmButton.textContent = options.confirmLabel;
    confirmButton.classList.toggle("danger", Boolean(options.danger));
    cancelButton.textContent = options.cancelLabel ?? "Cancel";
    cancelButton.hidden = !cancellable;
    closeButton.hidden = !cancellable;
    closeButton.setAttribute("aria-label", options.cancelLabel ?? "Close");
    dialog.dataset.cancellable = String(cancellable);

    const settle = (accepted: boolean): void => {
      dialog.removeEventListener("click", onClick);
      dialog.removeEventListener("cancel", onCancel);
      if (dialog.open) dialog.close();
      queueMicrotask(() => previousFocus?.focus());
      resolve(accepted);
    };
    const onCancel = (event: Event): void => {
      event.preventDefault();
      if (cancellable) settle(false);
    };
    const onClick = (event: MouseEvent): void => {
      const target = event.target;
      if (target === dialog && cancellable) {
        settle(false);
        return;
      }
      if (!(target instanceof Element)) return;
      const action = target.closest<HTMLElement>("[data-luna-dialog-action]")?.dataset
        .lunaDialogAction;
      if (action === "confirm") settle(true);
      else if (action === "cancel" && cancellable) settle(false);
    };

    dialog.addEventListener("click", onClick);
    dialog.addEventListener("cancel", onCancel);
    dialog.showModal();
    queueMicrotask(() => confirmButton.focus());
  }));
}

export function requestConfirmation(options: AppDialogOptions): Promise<boolean> {
  return presentDialog(options, true);
}

export async function showNotice(
  options: Omit<AppDialogOptions, "cancelLabel">,
): Promise<void> {
  await presentDialog(options, false);
}
