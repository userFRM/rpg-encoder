import { createSlice } from "@reduxjs/toolkit";
import { loginUser } from "./thunks";

interface AuthState {
    user: string | null;
    loading: boolean;
    error: string | null;
}

const initialState: AuthState = {
    user: null,
    loading: false,
    error: null,
};

const authSlice = createSlice({
    name: "auth",
    initialState,
    reducers: {
        loginStarted(state) {
            state.loading = true;
            state.error = null;
        },
        loginSucceeded(state, action) {
            state.loading = false;
            state.user = action.payload;
        },
        logout(state) {
            state.user = null;
            state.loading = false;
        },
    },
});

export const { loginStarted, loginSucceeded, logout } = authSlice.actions;
export default authSlice.reducer;
