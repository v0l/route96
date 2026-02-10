import { Outlet } from "react-router-dom";
import Header from "./views/header";

function App() {
  return (
    <div className="min-h-screen bg-black text-white">
      <Header />
      <main className="p-4">
        <Outlet />
      </main>
    </div>
  );
}

export default App;
