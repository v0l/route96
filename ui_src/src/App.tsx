import Header from "./views/header";
import Upload from "./views/upload";

function App() {
  return (
    <div className="flex flex-col gap-4 w-[700px] mx-auto mt-4">
      <Header />
      <Upload />
    </div>
  );
}

export default App;
