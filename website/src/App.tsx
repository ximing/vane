import { BrowserRouter, Route, Routes } from 'react-router-dom';
import { routes } from './routes';

export default function App() {
  return (
    <BrowserRouter basename="/vane">
      <Routes>
        {routes.map(({ path, Component }) => (
          <Route key={path} path={path} element={<Component />} />
        ))}
      </Routes>
    </BrowserRouter>
  );
}
