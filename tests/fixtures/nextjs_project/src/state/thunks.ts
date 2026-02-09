import { createAsyncThunk } from "@reduxjs/toolkit";

export const loginUser = createAsyncThunk(
    "auth/loginUser",
    async (credentials: { email: string; password: string }) => {
        const response = await fetch("/api/login", {
            method: "POST",
            body: JSON.stringify(credentials),
        });
        return response.json();
    }
);
