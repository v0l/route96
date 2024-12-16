import Header from "./views/header";
import Upload from "./views/upload";

function App() {
  return (
    <div className="flex flex-col gap-4 mx-auto mt-4 max-w-[1920px] px-10">
      <Header />
      <Upload />
    </div>
  );
}

export default App;
