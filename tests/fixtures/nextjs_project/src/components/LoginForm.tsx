import { loginRequested } from "../state/actions";
import { setAuthStore } from "../state/store";

export function LoginForm() {
    const value = "alice";
    setAuthStore(value);
    dispatch(loginRequested());
    return <form />;
}
