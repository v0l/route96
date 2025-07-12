import { Outlet } from "react-router-dom";
import Header from "./views/header";

function App() {
  return (
    <div className="min-h-screen bg-background text-foreground dark">
      <div className="max-lg:px-6">
        <Header />
        <main className="py-8">
          <Outlet />
        </main>
      </div>
    </div>
  );
}

export default App;
