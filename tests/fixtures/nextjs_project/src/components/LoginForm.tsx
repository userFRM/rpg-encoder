import { useSelector, useDispatch } from "react-redux";
import { selectAuthLoading } from "../state/selectors";
import { loginUser } from "../state/thunks";

export function LoginForm() {
    const loading = useSelector(selectAuthLoading);
    const dispatch = useDispatch();
    const handleSubmit = async (e: React.FormEvent) => {
        e.preventDefault();
        dispatch(loginUser({ email: "a@b.com", password: "123" }));
    };
    return (
        <form onSubmit={handleSubmit}>
            <button disabled={loading}>Log in</button>
        </form>
    );
}
