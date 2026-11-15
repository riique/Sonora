export type ViewKey = "inicio" | "historico" | "insights" | "ajustes";

export type SettingsTab = "geral" | "transcricao" | "vocabulario";

export type Navigate = (view: ViewKey, tab?: SettingsTab) => void;
