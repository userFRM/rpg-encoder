import { useSelector } from "react-redux";
import { LoginForm } from "../../src/components/LoginForm";
import { selectIsLoggedIn } from "../../src/state/selectors";

export default function Page() {
    const isLoggedIn = useSelector(selectIsLoggedIn);
    if (isLoggedIn) return <p>Already logged in</p>;
    return (
        <main>
            <LoginForm />
        </main>
    );
}
