const TOKEN_KEY = 'pertisk_token';
const USERNAME_KEY = 'pertisk_username';
const REMEMBER_KEY = 'pertisk_remember';
const SAVED_USER_KEY = 'pertisk_saved_username';
const SAVED_PASS_KEY = 'pertisk_saved_password';

export function getToken(): string | null {
  return localStorage.getItem(TOKEN_KEY);
}

export function setToken(token: string) {
  localStorage.setItem(TOKEN_KEY, token);
}

export function clearToken() {
  localStorage.removeItem(TOKEN_KEY);
  localStorage.removeItem(USERNAME_KEY);
}

export function getUsername(): string | null {
  return localStorage.getItem(USERNAME_KEY);
}

export function setUsername(username: string) {
  const trimmed = username.trim();
  if (trimmed) localStorage.setItem(USERNAME_KEY, trimmed);
  else localStorage.removeItem(USERNAME_KEY);
}

export function isRememberEnabled(): boolean {
  return localStorage.getItem(REMEMBER_KEY) === 'true';
}

export function getRememberedCredentials(): { username: string; password: string } | null {
  if (!isRememberEnabled()) return null;
  const username = localStorage.getItem(SAVED_USER_KEY)?.trim();
  const password = localStorage.getItem(SAVED_PASS_KEY) ?? '';
  if (!username) return null;
  return { username, password };
}

export function setRememberCredentials(username: string, password: string, remember: boolean) {
  if (remember) {
    localStorage.setItem(REMEMBER_KEY, 'true');
    localStorage.setItem(SAVED_USER_KEY, username.trim());
    localStorage.setItem(SAVED_PASS_KEY, password);
    return;
  }
  localStorage.removeItem(REMEMBER_KEY);
  localStorage.removeItem(SAVED_USER_KEY);
  localStorage.removeItem(SAVED_PASS_KEY);
}
