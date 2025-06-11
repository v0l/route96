import { BrowserRouter as Router, Routes, Route } from "react-router-dom";
import Header from "./views/header";
import Upload from "./views/upload";
import Admin from "./views/admin";

function App() {
  return (
    <Router>
      <div className="min-h-screen bg-gray-900">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8">
          <Header />
          <main className="py-8">
            <Routes>
              <Route path="/" element={<Upload />} />
              <Route path="/admin" element={<Admin />} />
            </Routes>
          </main>
        </div>
      </div>
    </Router>
  );
}

export default App;
