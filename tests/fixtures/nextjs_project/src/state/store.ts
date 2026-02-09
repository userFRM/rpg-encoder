import { configureStore } from "@reduxjs/toolkit";
import authReducer from "./authSlice";
import { postsApi } from "./api";

export const store = configureStore({
    reducer: {
        auth: authReducer,
        [postsApi.reducerPath]: postsApi.reducer,
    },
});

export type RootState = ReturnType<typeof store.getState>;
export type AppDispatch = typeof store.dispatch;
