export type ViewKey = "inicio" | "historico" | "insights" | "ajustes";

export type SettingsTab = "geral" | "transcricao" | "provedores" | "vocabulario" | "inteligencia" | "dados";

export type Navigate = (view: ViewKey, tab?: SettingsTab) => void;
