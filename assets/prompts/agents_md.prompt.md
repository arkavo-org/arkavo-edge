### Master Prompt for Generating a Tailored `AGENTS.MD` File

**Prompt Preamble:**
You are a configuration specialist tasked with creating a clear and effective `AGENTS.MD` file. This file will serve as the primary instruction set for a specialized AI agent interacting with a specific repository. Your goal is to generate a file that is perfectly tailored to the agent's purpose and capabilities, omitting any irrelevant information to maximize its efficiency.

**Instructions:**
Complete the following sections with the specific details of the agent and repository. Based on your inputs, I will generate the final `AGENTS.MD` file.

---

### **Step 1: Define the Agent's Core Context**

*   **Agent's Hyper-Specialized Purpose:**
    *(Describe the agent's primary goal in one or two sentences. Be specific.)*
    > **Example:** "This agent's purpose is to refactor legacy JavaScript code to modern TypeScript, ensuring all new code passes linting and unit tests."
    > **Example:** "This agent is designed to proofread Markdown documentation files, fix grammatical errors, and ensure all content conforms to the company's style guide."

*   **Agent Type:**
    *(Choose one: `CODING` or `NON-CODING`)*
    > **Example:** `CODING`

*   **Repository URL / Name:**
    *(Provide the name or URL of the repository the agent will work in.)*
    > **Example:** `https://github.com/my-org/project-phoenix`

---

### **Step 2: Detail the Repository and Workflow**

**(Instructions:** Based on the **Agent Type** you specified above, provide the relevant details below. **Omit any sections that are not applicable** to the agent's tasks.)

#### **`IF AGENT TYPE = CODING`**

*   **Project Overview:** *(Briefly describe the project's function.)*
*   **Project Structure:** *(List key directories/files and their purpose. e.g., `/src`, `/tests`, `Dockerfile`)*
*   **Development Setup & Commands:** *(List all commands for setup, building, testing, linting, etc. e.g., `npm install`, `npm run build`, `npm test`)*
*   **Coding Conventions:** *(Describe the coding style, formatting rules, or link to a style guide file.)*
*   **Architecture & Design Patterns:** *(Explain the high-level architecture or required design patterns. e.g., "This is a microservices architecture; new services must use the Repository pattern.")*
*   **Testing Guidelines:** *(Detail the testing philosophy, frameworks used, and where to add new tests.)*
*   **Contribution & PR Guidelines:** *(Describe commit message format and PR expectations.)*

#### **`IF AGENT TYPE = NON-CODING`**

*(Choose the most relevant template below or create your own sections.)*

*   **For a Documentation Agent:**
    *   **Content Overview:** *(What is the purpose of this documentation? Who is the audience?)*
    *   **Content Structure:** *(List key directories and their purpose, e.g., `/guides`, `/reference`, `/assets`)*
    *   **Style and Tone Guide:** *(Describe the writing style, tone, voice, and any formatting rules. Link to a style guide if available.)*
    *   **Validation Process:** *(List commands for linting text, checking for broken links, etc. e.g., `npx textlint .`)*
    *   **Contribution Guidelines:** *(How should changes be submitted?)*

*   **For a Data Analysis Agent:**
    *   **Project Goal:** *(What is the objective of the analysis in this repository?)*
    *   **Data Source Locations:** *(Where is raw data, processed data, notebooks, and reports located?)*
    *   **Tooling and Libraries:** *(Specify language, libraries, and environment setup commands. e.g., `Python 3.11`, `pandas`, `pip install -r requirements.txt`)*
    *   **Analysis & Reporting Guidelines:** *(Instructions on reproducibility, data cleaning, visualization standards, and report generation.)*

*   **For a General Purpose / Custom Agent:**
    *   *(Create custom headers that are relevant to your agent's task. For example: `Workflow Rules`, `File Handling Protocols`, `Communication Standards`, etc.)*

---

### **Step 3: Define Agent Capabilities and Constraints**

*   **Allowed Capabilities:**
    *(List the specific actions the agent is permitted to perform.)*
    > **Example:** `read files`, `write files`, `execute shell commands (npm test only)`, `submit pull requests`.
    > **Example:** `read files`, `write to files in the /docs directory only`.

*   **Data Access & Constraints:**
    *(Describe the data or directories the agent is allowed to access and any it is forbidden from touching.)*
    > **Example:** "The agent can only access files within the `/src` and `/tests` directories. It is forbidden from modifying any files in the `/.github` or `/config` directories."
    > **Example:** "The agent has read-only access to the `/data/raw` directory and read/write access to the `/data/processed` directory."

#### Step 3b: Arkavo Edge Runtime Configuration (Required)
Include the following runtime configuration, unchanged, in the final `AGENTS.MD` under a section titled “Runtime Configuration (Arkavo Edge)”.

- Model URI: `ollama://127.0.0.1:11434/qwen3:0.6b`
- Listen Address: `0.0.0.0:8342`

Also, render the configuration as YAML in the final file:
```
model:   ollama://127.0.0.1:11434/qwen3:0.6b
listen:  0.0.0.0:8342
```

---

### **Step 4: Generate the File**

**Final Instruction:** Based on all the information provided above, generate the complete and final `AGENTS.MD` file. The file should be well-structured, clear, concise, and contain only the relevant sections for the specified agent. Enclose the final output in a single Markdown code block.
