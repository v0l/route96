export default function FileList({ files }: { files: Array<File> }) {
  if (files.length === 0) {
    return <b>No Files</b>;
  }

  function renderInner(f: File) {
    if (f.type.startsWith("image/")) {
      return (
        <img src={URL.createObjectURL(f)} className="object-cover bg-center" />
      );
    }
  }
  return (
    <div className="grid grid-cols-4 gap-2">
      {files.map((a) => (
        <div
          key={a.name}
          className="relative rounded-md aspect-square overflow-hidden bg-neutral-800"
        >
          <div className="absolute flex flex-col items-center justify-center w-full h-full bg-black/50 text-wrap text-sm break-all text-center">
            {a.name}
          </div>
          {renderInner(a)}
        </div>
      ))}
    </div>
  );
}
