import { Navigate, useParams } from 'react-router-dom';

/**
 * Back-compat for links to the 0.68.0 standalone Debug page: `/debug` and
 * `/debug/:threadId` now land on the ordinary chat (`/chat`, `/chat/:threadId`),
 * where Debug Mode is the top-bar switch on the open thread.
 */
const DebugRedirect = () => {
  const { threadId } = useParams<{ threadId?: string }>();
  const target = threadId ? `/chat/${encodeURIComponent(threadId)}` : '/chat';
  return <Navigate to={target} replace />;
};

export default DebugRedirect;
