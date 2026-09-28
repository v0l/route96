export async function openFile(): Promise<File | undefined> {
  const files = await openFiles();
  return files?.[0];
}

export async function openFiles(): Promise<FileList | undefined> {
  return new Promise((resolve) => {
    const elm = document.createElement("input");
    elm.type = "file";
    elm.multiple = true;
    elm.addEventListener(
      "change",
      () => resolve((elm.files?.length ?? 0) > 0 ? elm.files! : undefined),
      { once: true },
    );
    elm.addEventListener("cancel", () => resolve(undefined), { once: true });
    elm.click();
  });
}
