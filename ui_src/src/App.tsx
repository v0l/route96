import { BrowserRouter as Router, Routes, Route } from "react-router-dom";
import Header from "./views/header";
import Upload from "./views/upload";
import Admin from "./views/admin";
import UserScope from "./views/user-scope";

function App() {
  return (
    <Router>
      <div className="min-h-screen bg-background text-foreground dark">
        <div className="max-lg:px-6">
          <Header />
          <main className="py-8">
            <Routes>
              <Route path="/" element={<Upload />} />
              <Route path="/admin" element={<Admin />} />
              <Route path="/admin/user/:pubkey" element={<UserScope />} />
            </Routes>
          </main>
        </div>
      </div>
    </Router>
  );
}

export default App;
