import { useSelector, useDispatch } from "react-redux";
import { selectUser } from "../state/selectors";
import { logout } from "../state/authSlice";

export const useAuth = () => {
    const user = useSelector(selectUser);
    const dispatch = useDispatch();
    return {
        user,
        logout: () => dispatch(logout()),
    };
};
