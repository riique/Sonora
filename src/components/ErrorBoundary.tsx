import { Component, type ErrorInfo, type ReactNode } from "react";

interface Props {
  children: ReactNode;
}

interface State {
  hasError: boolean;
  error: Error | null;
}

/**
 * Catches rendering errors anywhere in the child tree and displays a
 * fallback UI instead of a blank white screen. Without this, a single
 * component crash (e.g. a malformed prop from the backend) takes down
 * the entire app.
 */
export class ErrorBoundary extends Component<Props, State> {
  constructor(props: Props) {
    super(props);
    this.state = { hasError: false, error: null };
  }

  static getDerivedStateFromError(error: Error): State {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, info: ErrorInfo): void {
    console.error("ErrorBoundary caught an error:", error, info.componentStack);
  }

  handleReload = (): void => {
    this.setState({ hasError: false, error: null });
  };

  render(): ReactNode {
    if (this.state.hasError) {
      return (
        <div className="flex h-screen w-screen flex-col items-center justify-center gap-6 bg-canvas text-ink">
          <div className="flex flex-col items-center gap-3 text-center">
            <div className="flex h-16 w-16 items-center justify-center rounded-full bg-live-wash text-live">
              <svg
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.5"
                strokeLinecap="round"
                strokeLinejoin="round"
                className="h-8 w-8"
              >
                <path d="M12 9v4M12 17h.01M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
              </svg>
            </div>
            <h1 className="text-lg font-semibold">Algo deu errado</h1>
            <p className="max-w-md text-sm text-muted">
              Ocorreu um erro inesperado. Tente recarregar a página. Se o
              problema persistir, reinicie o aplicativo.
            </p>
            {this.state.error && (
              <pre className="mt-2 max-w-lg overflow-auto rounded-lg bg-fill p-4 text-xs text-strong">
                {this.state.error.message}
              </pre>
            )}
          </div>
          <button
            onClick={this.handleReload}
            className="rounded-[9px] bg-ink px-5 py-2 text-sm font-medium text-canvas transition-colors hover:bg-strong"
          >
            Tentar novamente
          </button>
        </div>
      );
    }

    return this.props.children;
  }
}
